//! The PvP view on a real capture.
//!
//! `packets_20261018_170140.txt` (A2_PVP_CAPTURE), 2026-10-18: Bradcrit logs
//! in, inspects two players, teleports to the Abyss and is killed there by
//! Ranger at 17:05:32, with MILFSHAKE and HuK, all three of the other faction.
//! No HP 0 came for him: his last was 625, then hits, then his revive.

use std::sync::Arc;

use a2tools_dps_meter_lib::capture::packet_accumulator::PacketAccumulator;
use a2tools_dps_meter_lib::capture::stream_processor::StreamProcessor;
use a2tools_dps_meter_lib::combat::data_storage::DataStorage;
use a2tools_dps_meter_lib::entity::job_class::JobClass;
use a2tools_dps_meter_lib::i18n::lookup::{NpcLookup, SkillLookup};
use a2tools_dps_meter_lib::share::read_capture;

#[test]
fn the_players_who_killed_you_are_the_attackers_on_top() {
    let Ok(path) = std::env::var("A2_PVP_CAPTURE") else {
        eprintln!("A2_PVP_CAPTURE unset; skipping");
        return;
    };
    let storage = Arc::new(DataStorage::new());
    let mut processor =
        StreamProcessor::new(storage.clone(), Arc::new(SkillLookup::new()), Arc::new(NpcLookup::new()));
    let mut acc = PacketAccumulator::new();
    let mut before_abyss = None;
    let mut at_death = None;
    let mut recap = None;
    for p in read_capture(std::path::Path::new(&path)).unwrap() {
        a2tools_dps_meter_lib::clock::set_override(Some(p.captured_at_ms));
        processor.set_override_timestamp(Some(p.captured_at_ms));
        acc.append(&p.bytes);
        let used = processor.consume_stream(acc.snapshot());
        if used > 0 {
            acc.discard_bytes(used);
        }
        let clock = chrono::DateTime::from_timestamp_millis(p.captured_at_ms).unwrap().with_timezone(&chrono::FixedOffset::east_opt(7200).unwrap());
        let hms = clock.format("%H:%M:%S").to_string();
        if before_abyss.is_none() && hms.as_str() >= "17:04:00" {
            assert!(!storage.in_abyss(), "the home map");
            before_abyss = Some(storage.nearby_enemies());
        }
        // The engine asks every half second or so; the revive's HP came 24 s on.
        recap = recap.or_else(|| storage.last_pvp_death());
        if at_death.is_none() && hms.as_str() >= "17:05:31" {
            assert!(storage.in_abyss(), "teleported to the Abyss at 17:04:22");
            at_death = Some(storage.nearby_enemies());
        }
    }
    a2tools_dps_meter_lib::clock::set_override(None);

    // Before the Abyss, everyone around was of your faction.
    assert!(before_abyss.unwrap().is_empty());

    let enemies = at_death.unwrap();
    let attackers: Vec<_> = enemies.iter().take_while(|p| p.attacked_me_ms.is_some()).map(|p| p.name.as_str()).collect();
    let mut sorted = attackers.clone();
    sorted.sort();
    assert_eq!(sorted, ["HuK", "MILFSHAKE", "Ranger"], "{enemies:#?}");
    let ranger = enemies.iter().find(|p| p.name == "Ranger").unwrap();
    assert_eq!(ranger.class, Some(JobClass::Ranger));
    assert!(enemies.iter().all(|p| p.faction == 1));
    assert!(enemies.iter().filter(|p| p.attacked_me_ms.is_some()).all(|p| p.taken > 0));

    // The death recap: who killed you, and with what.
    let recap = recap.expect("a death recap");
    let mut names: Vec<_> = recap.attackers.iter().map(|a| a.name.as_str()).collect();
    names.sort();
    assert_eq!(names, ["HuK", "MILFSHAKE", "Ranger"]);
    assert_eq!(recap.killer, "Ranger");
}
