//! How densely a capture tells where everyone is — the question behind a
//! dungeon replay (positions of you, other players, bosses and mobs over a
//! fight, and the spots of NPC skill casts).
//!
//! Run with `A2_DENSITY_CAPTURE=<packets_*.txt> cargo test --release --test replay_density_scan -- --nocapture`.
//! Prints, per boss seen: its time window, and inside it for each entity class
//! how many entities, how many spots, and the gaps between one entity's spots
//! (median / p90 / max); plus NPC casts that carry a spot.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use a2tools_dps_meter_lib::capture::frame_tap;
use a2tools_dps_meter_lib::capture::packet_accumulator::PacketAccumulator;
use a2tools_dps_meter_lib::capture::positions::{self, Position, Spotted};
use a2tools_dps_meter_lib::capture::stream_processor::{read_varint, StreamProcessor};
use a2tools_dps_meter_lib::combat::data_storage::DataStorage;
use a2tools_dps_meter_lib::i18n::lookup::{NpcLookup, SkillLookup};
use a2tools_dps_meter_lib::share::read_capture;

#[derive(Default)]
struct Scan {
    /// Entity → (time, frame type, spot).
    spots: HashMap<i32, Vec<(i64, String, Position)>>,
    /// Your spots (entity 0 / local id), merged.
    mine: Vec<(i64, Position)>,
    /// `02 38` with a spot: (time, caster, skill, target, spot).
    casts: Vec<(i64, i32, u32, i32, Position)>,
}

fn quantile(v: &mut [i64], q: f64) -> i64 {
    if v.is_empty() {
        return -1;
    }
    v.sort_unstable();
    v[((v.len() - 1) as f64 * q).round() as usize]
}

#[test]
fn replay_density() {
    let Ok(path) = std::env::var("A2_DENSITY_CAPTURE") else {
        eprintln!("A2_DENSITY_CAPTURE unset; skipping");
        return;
    };
    let storage = Arc::new(DataStorage::new());
    let scan = Arc::new(Mutex::new(Scan::default()));
    {
        let storage = storage.clone();
        let scan = scan.clone();
        frame_tap::set(Some(Arc::new(move |packet: &[u8]| {
            let len = read_varint(packet, 0);
            if len.length <= 0 {
                return;
            }
            let at = len.length as usize;
            let Some(op) = packet.get(at..at + 2) else { return };
            let kind = format!("{:02X} {:02X}", op[0], op[1]);
            let now = a2tools_dps_meter_lib::clock::now_ms();
            let me = storage.local_player_id();
            let mut scan = scan.lock();
            match positions::parse(packet) {
                Some(Spotted::You(pos)) => scan.mine.push((now, pos)),
                Some(Spotted::Entity(id, pos)) if Some(id as i64) == me => scan.mine.push((now, pos)),
                Some(Spotted::Entity(id, pos)) => scan.spots.entry(id).or_default().push((now, kind.clone(), pos)),
                None => {}
            }
            if op == [0x02, 0x38] {
                if let Some(Spotted::Entity(target, pos)) = positions::parse(packet) {
                    let caster = read_varint(packet, at + 2);
                    let i = at + 2 + caster.length as usize + 1;
                    if let Some(b) = packet.get(i..i + 4) {
                        let skill = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
                        scan.casts.push((now, caster.value, skill, target, pos));
                    }
                }
            }
        })));
    }

    let mut processor = StreamProcessor::new(storage.clone(), Arc::new(SkillLookup::new()), Arc::new(NpcLookup::new()));
    let mut streams: HashMap<String, PacketAccumulator> = HashMap::new();
    let mut first = None;
    let mut last = 0;
    for p in read_capture(std::path::Path::new(&path)).unwrap() {
        a2tools_dps_meter_lib::clock::set_override(Some(p.captured_at_ms));
        processor.set_override_timestamp(Some(p.captured_at_ms));
        first.get_or_insert(p.captured_at_ms);
        last = p.captured_at_ms;
        let acc = streams.entry(p.stream.clone()).or_insert_with(PacketAccumulator::new);
        acc.append(&p.bytes);
        let used = processor.consume_stream(acc.snapshot());
        if used > 0 {
            acc.discard_bytes(used);
        }
    }
    frame_tap::set(None);
    a2tools_dps_meter_lib::clock::set_override(None);

    let scan = scan.lock();
    let clock = |t: i64| chrono::DateTime::from_timestamp_millis(t).unwrap().with_timezone(&chrono::Local).format("%H:%M:%S").to_string();
    eprintln!("capture {} – {} ({} s)", clock(first.unwrap_or(0)), clock(last), (last - first.unwrap_or(0)) / 1000);

    // Windows: one per boss entity, first to last spot.
    let mut windows: Vec<(i32, i64, i64)> = scan
        .spots
        .iter()
        .filter(|(id, _)| storage.is_boss(**id))
        .map(|(id, v)| (*id, v.first().unwrap().0, v.last().unwrap().0))
        .collect();
    windows.sort_by_key(|w| w.1);
    if windows.is_empty() {
        eprintln!("no boss seen; whole capture as one window");
        windows.push((0, first.unwrap_or(0), last));
    }

    let class = |id: i32| {
        if storage.is_boss(id) {
            "boss"
        } else if storage.is_player(id) {
            "player"
        } else if storage.is_mob(id) {
            "mob"
        } else {
            "other"
        }
    };

    for (boss, from, to) in &windows {
        eprintln!(
            "\n=== boss {boss} (code {}) {} – {} ({} s)",
            storage.mob_code(*boss).unwrap_or(-1),
            clock(*from),
            clock(*to),
            (to - from) / 1000
        );
        let inside = |t: i64| t >= *from && t <= *to;
        let mut mine: Vec<i64> = scan.mine.iter().filter(|(t, _)| inside(*t)).map(|(t, _)| *t).collect::<Vec<_>>();
        let n_mine = mine.len();
        let mut mine_gaps: Vec<i64> = mine.windows(2).map(|w| w[1] - w[0]).collect();
        eprintln!(
            "  you: {n_mine} spots, gap median {} p90 {} max {} ms",
            quantile(&mut mine_gaps.clone(), 0.5),
            quantile(&mut mine_gaps.clone(), 0.9),
            quantile(&mut mine_gaps, 1.0)
        );
        mine.clear();

        let mut by_class: HashMap<&str, (u32, usize, Vec<i64>, HashMap<String, usize>)> = HashMap::new();
        for (id, v) in &scan.spots {
            let ts: Vec<&(i64, String, Position)> = v.iter().filter(|(t, ..)| inside(*t)).collect();
            if ts.is_empty() {
                continue;
            }
            let e = by_class.entry(class(*id)).or_default();
            e.0 += 1;
            e.1 += ts.len();
            e.2.extend(ts.windows(2).map(|w| w[1].0 - w[0].0));
            for (_, k, _) in &ts {
                *e.3.entry(k.clone()).or_default() += 1;
            }
            if class(*id) == "player" || class(*id) == "boss" {
                let mut g: Vec<i64> = ts.windows(2).map(|w| w[1].0 - w[0].0).collect();
                let span = ts.last().unwrap().0 - ts.first().unwrap().0;
                eprintln!(
                    "    {} {id}: {} spots over {} s, gap median {} p90 {} max {} ms",
                    class(*id),
                    ts.len(),
                    span / 1000,
                    quantile(&mut g.clone(), 0.5),
                    quantile(&mut g.clone(), 0.9),
                    quantile(&mut g, 1.0)
                );
            }
        }
        let mut classes: Vec<_> = by_class.into_iter().collect();
        classes.sort_by_key(|(c, _)| *c);
        for (c, (n, spots, mut gaps, kinds)) in classes {
            let mut kinds: Vec<_> = kinds.into_iter().collect();
            kinds.sort_by(|a, b| b.1.cmp(&a.1));
            eprintln!(
                "  {c}: {n} entities, {spots} spots, gap median {} p90 {} ms; by frame {:?}",
                quantile(&mut gaps.clone(), 0.5),
                quantile(&mut gaps, 0.9),
                kinds
            );
        }
        let mut npc_casts: HashMap<(i32, u32), u32> = HashMap::new();
        for (t, caster, skill, _, _) in &scan.casts {
            if inside(*t) && !storage.is_player(*caster) {
                *npc_casts.entry((storage.mob_code(*caster).unwrap_or(-1), *skill)).or_default() += 1;
            }
        }
        let mut npc_casts: Vec<_> = npc_casts.into_iter().collect();
        npc_casts.sort_by(|a, b| b.1.cmp(&a.1));
        eprintln!("  NPC casts with a spot (caster code, skill) × n: {:?}", npc_casts.iter().take(15).collect::<Vec<_>>());
    }
}
