//! Your PvP kills on a real capture.
//!
//! `packets_20261018_212409.txt` (A2_KILL_CAPTURE), 2026-10-18: the meter
//! starts with Bradcrit already in the Abyss — no self record, so no faction,
//! until he leaves; the engine has the one he had last time (factions.json,
//! 2 since 17:01 that day), as here. He kills Yasiumaru (21:25:25, his hit the last before
//! HP 0) and Morgulia (21:25:38, another player's 75 and then his 522).

use std::sync::Arc;

use a2tools_dps_meter_lib::capture::packet_accumulator::PacketAccumulator;
use a2tools_dps_meter_lib::capture::stream_processor::StreamProcessor;
use a2tools_dps_meter_lib::combat::data_storage::DataStorage;
use a2tools_dps_meter_lib::i18n::lookup::{NpcLookup, SkillLookup};
use a2tools_dps_meter_lib::share::read_capture;

#[test]
fn both_kills_in_the_abyss_with_the_last_hit_yours() {
    let Ok(path) = std::env::var("A2_KILL_CAPTURE") else {
        eprintln!("A2_KILL_CAPTURE unset; skipping");
        return;
    };
    let storage = Arc::new(DataStorage::new());
    let mut processor =
        StreamProcessor::new(storage.clone(), Arc::new(SkillLookup::new()), Arc::new(NpcLookup::new()));
    storage.guess_self_faction(2);
    let mut acc = PacketAccumulator::new();
    let mut in_sight = 0;
    for p in read_capture(std::path::Path::new(&path)).unwrap() {
        a2tools_dps_meter_lib::clock::set_override(Some(p.captured_at_ms));
        processor.set_override_timestamp(Some(p.captured_at_ms));
        acc.append(&p.bytes);
        let used = processor.consume_stream(acc.snapshot());
        if used > 0 {
            acc.discard_bytes(used);
        }
        in_sight = in_sight.max(storage.nearby_enemies().len());
    }
    let kills = storage.pvp_kills();
    a2tools_dps_meter_lib::clock::set_override(None);

    assert!(in_sight > 0, "enemies in sight, though no self record named the faction");
    let rows: Vec<_> = kills.iter().map(|k| (k.name.as_str(), k.killing_blow)).collect();
    assert_eq!(rows, [("Yasiumaru", true), ("Morgulia", true)], "{kills:#?}");
    assert!(kills.iter().all(|k| k.damage > 0 && k.total >= k.damage));
}
