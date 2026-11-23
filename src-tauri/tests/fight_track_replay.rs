//! Replays a capture through the meter, saving boss fights as History does,
//! and prints each fight's track (`combat::fight_track`): who is in it and how
//! many spots each has.
//!
//! Run with `A2_TRACK_CAPTURE=<packets_*.txt> cargo test --release --test fight_track_replay -- --nocapture`.
//! `A2_TRACK_OUT=<dir>` also writes each fight as History would (`<id>.json`),
//! for the Companion's replay view.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;

use parking_lot::Mutex;

use a2tools_dps_meter_lib::capture::packet_accumulator::PacketAccumulator;
use a2tools_dps_meter_lib::capture::stream_processor::StreamProcessor;
use a2tools_dps_meter_lib::combat::data_storage::DataStorage;
use a2tools_dps_meter_lib::combat::dps_calculator::DpsCalculator;
use a2tools_dps_meter_lib::combat::ping_tracker::PingTracker;
use a2tools_dps_meter_lib::entity::fight_record::FightRecord;
use a2tools_dps_meter_lib::i18n::lookup::{NpcLookup, SkillLookup};
use a2tools_dps_meter_lib::share::read_capture;

#[test]
fn fight_tracks() {
    let Ok(path) = std::env::var("A2_TRACK_CAPTURE") else {
        eprintln!("A2_TRACK_CAPTURE unset; skipping");
        return;
    };
    let data = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/data");
    let npc = Arc::new(NpcLookup::new());
    npc.load_from_json(&std::fs::read_to_string(data.join("i18n/npcs/en.json")).unwrap());
    let sk = Arc::new(SkillLookup::new());
    sk.load_from_json(&std::fs::read_to_string(data.join("i18n/skills/en.json")).unwrap());
    let dots: Vec<i32> = serde_json::from_str(&std::fs::read_to_string(data.join("dot_skill_ids.json")).unwrap()).unwrap();

    let storage = Arc::new(DataStorage::new());
    let mut proc = StreamProcessor::new(storage.clone(), sk.clone(), npc.clone());
    proc.set_dot_skill_ids(dots.into_iter().collect());
    let calc = Arc::new(Mutex::new(DpsCalculator::new(storage.clone(), sk, npc, Arc::new(PingTracker::new()))));
    calc.lock().set_target_selection_mode("bossTargets");
    let fights: Arc<Mutex<BTreeMap<String, FightRecord>>> = Arc::default();
    let save = {
        let (calc, fights) = (calc.clone(), fights.clone());
        Arc::new(move |force: bool| {
            let records = if force { calc.lock().snapshot_boss_fights_force() } else { calc.lock().snapshot_boss_fights() };
            for r in records {
                fights.lock().insert(r.id.clone(), r);
            }
        })
    };
    {
        let save = save.clone();
        storage.set_before_reset(move || save(true));
    }

    let packets = read_capture(Path::new(&path)).expect("capture readable");
    let mut streams: HashMap<String, PacketAccumulator> = HashMap::new();
    let mut next_tick = i64::MIN;
    for p in &packets {
        a2tools_dps_meter_lib::clock::set_override(Some(p.captured_at_ms));
        proc.set_override_timestamp(Some(p.captured_at_ms));
        let acc = streams.entry(p.stream.clone()).or_insert_with(PacketAccumulator::new);
        acc.append(&p.bytes);
        let used = proc.consume_stream(acc.snapshot());
        if used > 0 {
            acc.discard_bytes(used);
        }
        if p.captured_at_ms >= next_tick {
            next_tick = p.captured_at_ms + 1_000;
            let _ = calc.lock().get_dps();
            save(false);
        }
    }
    save(true);
    a2tools_dps_meter_lib::clock::set_override(None);

    let out = std::env::var("A2_TRACK_OUT").ok();
    for (id, f) in fights.lock().iter() {
        eprintln!(
            "\n{id} {} (code {}) {} s, dungeon {}, killed {:?}",
            f.boss_name,
            f.mob_code,
            f.duration_ms / 1000,
            f.dungeon_id,
            f.killed
        );
        match &f.track {
            None => eprintln!("  no track"),
            Some(t) => {
                for e in &t.ents {
                    let n = e.pts.split(';').count();
                    eprintln!("  {:<6} {:>6} code {:>8} {:>5} spots", e.kind, e.id, e.code, n);
                }
                eprintln!("  casts {}", if t.casts.is_empty() { 0 } else { t.casts.split(';').count() });
            }
        }
        if let Some(dir) = &out {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(Path::new(dir).join(format!("{id}.json")), serde_json::to_string_pretty(f).unwrap()).unwrap();
        }
    }
}
