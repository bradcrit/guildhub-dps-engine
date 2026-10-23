//! Your spot and the mobs' distances on a real capture.
//!
//! `packets_20261023_223205.txt` (A2_POSITION_CAPTURE), 2026-10-23: a
//! teleport at 22:32:21, a run, kills at ~22:32:40, :45 and :50, a run with a
//! turn, a kill at ~22:33:07, a run straight on.

use std::sync::Arc;

use a2tools_dps_meter_lib::capture::packet_accumulator::PacketAccumulator;
use a2tools_dps_meter_lib::capture::stream_processor::StreamProcessor;
use a2tools_dps_meter_lib::combat::data_storage::DataStorage;
use a2tools_dps_meter_lib::i18n::lookup::{NpcLookup, SkillLookup};
use a2tools_dps_meter_lib::share::read_capture;

#[test]
fn your_spot_follows_the_run_and_the_kills() {
    let Ok(path) = std::env::var("A2_POSITION_CAPTURE") else {
        eprintln!("A2_POSITION_CAPTURE unset; skipping");
        return;
    };
    let storage = Arc::new(DataStorage::new());
    let mut processor =
        StreamProcessor::new(storage.clone(), Arc::new(SkillLookup::new()), Arc::new(NpcLookup::new()));
    let mut acc = PacketAccumulator::new();
    let mut spots = Vec::new();
    let mut last_second = 0;
    for p in read_capture(std::path::Path::new(&path)).unwrap() {
        a2tools_dps_meter_lib::clock::set_override(Some(p.captured_at_ms));
        processor.set_override_timestamp(Some(p.captured_at_ms));
        acc.append(&p.bytes);
        let used = processor.consume_stream(acc.snapshot());
        if used > 0 {
            acc.discard_bytes(used);
        }
        if p.captured_at_ms / 1000 == last_second {
            continue;
        }
        last_second = p.captured_at_ms / 1000;
        let clock = chrono::DateTime::from_timestamp_millis(p.captured_at_ms).unwrap().with_timezone(&chrono::FixedOffset::east_opt(7200).unwrap());
        let nearest = storage.mobs_in_sight(60_000).values().filter_map(|(_, m)| *m).fold(f32::MAX, f32::min);
        if let Some((pos, age)) = storage.self_position() {
            eprintln!("{} you ({:.0}, {:.0}) {:>5} ms old, nearest mob {:.1} m", clock.format("%H:%M:%S"), pos.x, pos.y, age, nearest);
            spots.push((clock.format("%H:%M:%S").to_string(), pos, age, nearest));
        }
    }
    a2tools_dps_meter_lib::clock::set_override(None);

    // The teleport puts you there right away.
    assert!(spots.iter().any(|(t, pos, ..)| t.as_str() >= "22:32:21" && (pos.x - 188_748.0).abs() < 1.0));
    // At each kill a mob is within skill range.
    for at in ["22:32:41", "22:33:08"] {
        let (_, _, age, nearest) = spots.iter().find(|(t, ..)| t.as_str() >= at).unwrap();
        assert!(*age < 2_000 && *nearest < 10.0, "{at}: {age} ms old, {nearest} m");
    }
}
