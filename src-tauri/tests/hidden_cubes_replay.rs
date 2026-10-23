//! The Hidden Cubes in range on a real capture.
//!
//! `packets_20261024_003648.txt` (A2_CUBE_CAPTURE), 2026-10-24: 18 minutes
//! out in the open world with no cube, then a glide down to one on a
//! platform at ~00:54:02 and opening it at ~00:54:49.

use std::sync::Arc;

use a2tools_dps_meter_lib::capture::packet_accumulator::PacketAccumulator;
use a2tools_dps_meter_lib::capture::stream_processor::StreamProcessor;
use a2tools_dps_meter_lib::combat::data_storage::DataStorage;
use a2tools_dps_meter_lib::i18n::lookup::{NpcLookup, SkillLookup};
use a2tools_dps_meter_lib::share::read_capture;

#[test]
fn the_cube_is_in_range_until_opened() {
    let Ok(path) = std::env::var("A2_CUBE_CAPTURE") else {
        eprintln!("A2_CUBE_CAPTURE unset; skipping");
        return;
    };
    let storage = Arc::new(DataStorage::new());
    let mut processor =
        StreamProcessor::new(storage.clone(), Arc::new(SkillLookup::new()), Arc::new(NpcLookup::new()));
    let mut acc = PacketAccumulator::new();
    let mut seconds = Vec::new();
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
        seconds.push((clock.format("%H:%M:%S").to_string(), storage.hidden_cubes(5_000)));
    }
    a2tools_dps_meter_lib::clock::set_override(None);

    let at = |t: &str| &seconds.iter().find(|(s, _)| s.as_str() >= t).unwrap().1;
    // None before it, one from ~00:54:02 on.
    assert!(seconds.iter().filter(|(s, _)| s.as_str() < "00:54:02").all(|(_, cubes)| cubes.is_empty()));
    assert_eq!(at("00:54:04").len(), 1);
    // Next to it on the platform: a few metres, the same height.
    let (metres, above) = at("00:54:08")[0].unwrap();
    assert!(metres < 10.0 && above.abs() < 2.0, "{metres} m, {above} m above");
    // Opened: gone.
    assert!(at("00:54:50").is_empty());
}
