//! Entry counts and currencies (`capture::game_lists`) replayed from the
//! capture of 2026-10-18 00:53, with what the game's windows showed:
//!
//! - 00:55 login; Abyss window 82,325 points; Shugo keys 19 (seen at 01:06)
//! - 00:56 a Stigma shard bought: 72,325
//! - 00:57 Nightmare 0(+9)/14, one run, then 0(+8)/14
//!
//!   A2_LISTS_CAPTURE=…/packets_20261018_005340.txt cargo test --test game_lists_replay

use std::sync::Arc;

use a2tools_dps_meter_lib::capture::packet_accumulator::PacketAccumulator;
use a2tools_dps_meter_lib::capture::stream_processor::StreamProcessor;
use a2tools_dps_meter_lib::combat::data_storage::{DataStorage, GameLists};
use a2tools_dps_meter_lib::i18n::lookup::{NpcLookup, SkillLookup};

fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok()).collect()
}

/// The lists after every packet before `until` (an ISO-8601 prefix; "" all).
fn replay(lines: &[(String, Vec<u8>)], until: &str) -> GameLists {
    let storage = Arc::new(DataStorage::new());
    let mut processor = StreamProcessor::new(storage.clone(), Arc::new(SkillLookup::new()), Arc::new(NpcLookup::new()));
    let mut acc = PacketAccumulator::new();
    for (ts, bytes) in lines {
        if !until.is_empty() && ts.as_str() >= until {
            break;
        }
        acc.append(bytes);
        let consumed = processor.consume_stream(acc.snapshot());
        if consumed > 0 {
            acc.discard_bytes(consumed);
        }
    }
    storage.game_lists()
}

fn count(lists: &GameLists, id: u32) -> (u32, u32, Option<u64>) {
    let e = lists.counts.get(&id).unwrap_or_else(|| panic!("entry {id}"));
    (e.first, e.second, e.ms)
}

#[test]
fn nightmare_shugo_abyss_as_the_windows_showed() {
    let Ok(path) = std::env::var("A2_LISTS_CAPTURE") else {
        eprintln!("A2_LISTS_CAPTURE unset; skipping");
        return;
    };
    let text = std::fs::read_to_string(path).expect("capture readable");
    let lines: Vec<(String, Vec<u8>)> = text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let mut parts = l.splitn(3, '|');
            let ts = parts.next()?.to_string();
            let _stream = parts.next()?;
            Some((ts, decode_hex(parts.next()?)?))
        })
        .collect();

    let login = replay(&lines, "2026-10-18T00:56:00");
    assert_eq!(count(&login, 4), (0, 9, None), "Nightmare 0(+9)");
    assert_eq!(count(&login, 1), (19, 0, None), "Shugo keys 19");
    assert_eq!(count(&login, 3).2, Some(19_670_000), "Lower Reshanta 5:27:50 left");
    assert_eq!(login.items.get(&930_100_003), Some(&82_325), "Abyss points");
    assert_eq!(login.items.get(&930_100_031), Some(&915_229), "Kinah");
    assert!(login.owner.is_some(), "the self record after the login named whose");

    let bought = replay(&lines, "2026-10-18T00:57:00");
    assert_eq!(bought.items.get(&930_100_003), Some(&72_325), "Abyss points after the shard");

    let run = replay(&lines, "");
    assert_eq!(count(&run, 4), (0, 8, None), "Nightmare 0(+8)");
    assert_eq!(run.items.get(&930_100_017), Some(&7_260), "Nightmare coins after the run");
    assert_eq!(run.owner, login.owner);
}

/// A second character, 2026-10-18 01:34: the top bar showed 321,520 (Kina and
/// Kina (Bound) together) and 1,920 (Kina).
///
///   A2_LISTS_CAPTURE_ALT=…/packets_20261018_013448.txt cargo test --test game_lists_replay
#[test]
fn kina_and_kina_bound_apart() {
    let Ok(path) = std::env::var("A2_LISTS_CAPTURE_ALT") else {
        eprintln!("A2_LISTS_CAPTURE_ALT unset; skipping");
        return;
    };
    let text = std::fs::read_to_string(path).expect("capture readable");
    let lines: Vec<(String, Vec<u8>)> = text
        .lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let mut parts = l.splitn(3, '|');
            let ts = parts.next()?.to_string();
            let _stream = parts.next()?;
            Some((ts, decode_hex(parts.next()?)?))
        })
        .collect();
    let lists = replay(&lines, "");
    assert!(lists.items_whole);
    assert_eq!(lists.items.get(&930_100_031), Some(&1_920), "Kina");
    assert_eq!(lists.items.get(&930_100_001), Some(&319_600), "Kina (Bound)");
}
