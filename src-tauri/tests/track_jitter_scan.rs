//! Which records make a replay's players jump: each player's spots by the
//! frame type that told them, against the path their move records
//! (`1A/1B/1C/3C 37`) draw — how far each other type's spot is from where the
//! moves put them then.
//!
//! Run with `A2_JITTER_CAPTURE=<packets_*.txt> cargo test --release --test track_jitter_scan -- --nocapture`.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use parking_lot::Mutex;

use a2tools_dps_meter_lib::capture::frame_tap;
use a2tools_dps_meter_lib::capture::packet_accumulator::PacketAccumulator;
use a2tools_dps_meter_lib::capture::positions::{self, Position, Spotted};
use a2tools_dps_meter_lib::capture::stream_processor::{read_varint, StreamProcessor};
use a2tools_dps_meter_lib::combat::data_storage::DataStorage;
use a2tools_dps_meter_lib::i18n::lookup::{NpcLookup, SkillLookup};
use a2tools_dps_meter_lib::share::read_capture;

fn quantiles(v: &mut [f32]) -> String {
    if v.is_empty() {
        return "-".into();
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let q = |f: f32| v[((v.len() - 1) as f32 * f).round() as usize];
    format!("n {:>5}  median {:>5.1} m  p75 {:>5.1}  p90 {:>5.1}  max {:>6.1}", v.len(), q(0.5), q(0.75), q(0.9), q(1.0))
}

#[test]
fn track_jitter() {
    let Ok(path) = std::env::var("A2_JITTER_CAPTURE") else {
        eprintln!("A2_JITTER_CAPTURE unset; skipping");
        return;
    };
    let storage = Arc::new(DataStorage::new());
    // entity → (time, kind, spot, effect/skill flags byte for 2A 38)
    let spots: Arc<Mutex<HashMap<i32, Vec<(i64, String, Position)>>>> = Arc::default();
    {
        let storage = storage.clone();
        let spots = spots.clone();
        frame_tap::set(Some(Arc::new(move |packet: &[u8]| {
            let len = read_varint(packet, 0);
            if len.length <= 0 {
                return;
            }
            let at = len.length as usize;
            let Some(op) = packet.get(at..at + 2) else { return };
            let kind = match op {
                [0x1A | 0x1B | 0x1C | 0x3C, 0x37] => "move".to_string(),
                _ => format!("{:02X} {:02X}", op[0], op[1]),
            };
            let now = a2tools_dps_meter_lib::clock::now_ms();
            let me = storage.local_player_id();
            match positions::parse(packet) {
                Some(Spotted::Entity(id, pos)) => {
                    let id = if Some(id as i64) == me { -1 } else { id };
                    spots.lock().entry(id).or_default().push((now, kind, pos));
                }
                Some(Spotted::You(pos)) => spots.lock().entry(-1).or_default().push((now, kind, pos)),
                None => {}
            }
        })));
    }
    let mut processor = StreamProcessor::new(storage.clone(), Arc::new(SkillLookup::new()), Arc::new(NpcLookup::new()));
    let mut streams: HashMap<String, PacketAccumulator> = HashMap::new();
    for p in read_capture(std::path::Path::new(&path)).unwrap() {
        a2tools_dps_meter_lib::clock::set_override(Some(p.captured_at_ms));
        processor.set_override_timestamp(Some(p.captured_at_ms));
        let acc = streams.entry(p.stream.clone()).or_insert_with(PacketAccumulator::new);
        acc.append(&p.bytes);
        let used = processor.consume_stream(acc.snapshot());
        if used > 0 {
            acc.discard_bytes(used);
        }
    }
    frame_tap::set(None);
    a2tools_dps_meter_lib::clock::set_override(None);

    let spots = spots.lock();
    let mut off: BTreeMap<String, Vec<f32>> = BTreeMap::new();
    let mut step: BTreeMap<String, Vec<f32>> = BTreeMap::new();
    for (id, v) in spots.iter() {
        if *id != -1 && !storage.is_player(*id) {
            continue;
        }
        let who = if *id == -1 { "you" } else { "player" };
        let moves: Vec<&(i64, String, Position)> = v.iter().filter(|s| s.1 == "move").collect();
        // Against the moves: only where a move came within 400 ms either side.
        for (t, kind, pos) in v.iter() {
            if kind == "move" {
                continue;
            }
            let i = moves.partition_point(|m| m.0 <= *t);
            let (Some(a), Some(b)) = (i.checked_sub(1).and_then(|i| moves.get(i)), moves.get(i)) else { continue };
            if t - a.0 > 400 || b.0 - t > 400 {
                continue;
            }
            let k = if b.0 > a.0 { (t - a.0) as f32 / (b.0 - a.0) as f32 } else { 0.0 };
            let there = Position { x: a.2.x + (b.2.x - a.2.x) * k, y: a.2.y + (b.2.y - a.2.y) * k, z: a.2.z };
            off.entry(format!("{who} {kind}")).or_default().push(pos.metres_to(&there));
        }
        // Each spot against the one before (any kind), when < 300 ms apart: a jump.
        for w in v.windows(2) {
            if w[1].0 - w[0].0 <= 300 {
                step.entry(format!("{who} {} after {}", w[1].1, w[0].1)).or_default().push(w[1].2.metres_to(&w[0].2));
            }
        }
    }
    eprintln!("\nOff the moves' path, by the record that told the spot:");
    for (k, v) in off.iter_mut() {
        eprintln!("  {k:<18} {}", quantiles(v));
    }
    eprintln!("\nJump from the spot before (≤ 300 ms apart):");
    for (k, v) in step.iter_mut() {
        eprintln!("  {k:<30} {}", quantiles(v));
    }
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for (id, v) in spots.iter() {
        if *id == -1 || storage.is_player(*id) {
            for s in v {
                *counts.entry(format!("{} {}", if *id == -1 { "you" } else { "player" }, s.1)).or_default() += 1;
            }
        }
    }
    eprintln!("\nSpots by record: {counts:?}");
}
