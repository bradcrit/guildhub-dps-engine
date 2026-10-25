//! Replays a packet capture and prints, per healer, the healing the game
//! reported against the effective healing (`effective_heal`: up to what the
//! target was missing of their max HP) — the overheal in between.
//!
//! What it checks: a heal is never counted as more than it reported, and the
//! effective share is neither ~0 (the HP after a heal arriving before it would
//! make every heal look like overheal) nor always 100 % (the HP unknown).
//!
//!   A2_OVERHEAL_CAPTURE=<capture> cargo test --test overheal_replay -- --nocapture

use std::collections::HashMap;
use std::sync::Arc;

use a2tools_dps_meter_lib::capture::packet_accumulator::PacketAccumulator;
use a2tools_dps_meter_lib::capture::stream_processor::StreamProcessor;
use a2tools_dps_meter_lib::combat::data_storage::DataStorage;
use a2tools_dps_meter_lib::i18n::lookup::{NpcLookup, SkillLookup};

fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok()).collect()
}

#[test]
fn effective_healing_is_at_most_the_reported() {
    let Ok(path) = std::env::var("A2_OVERHEAL_CAPTURE") else {
        eprintln!("A2_OVERHEAL_CAPTURE unset — skipping");
        return;
    };
    let storage = Arc::new(DataStorage::new());
    let mut processor = StreamProcessor::new(storage.clone(), Arc::new(SkillLookup::new()), Arc::new(NpcLookup::new()));
    let text = std::fs::read_to_string(&path).expect("capture readable");
    let mut streams: HashMap<String, PacketAccumulator> = HashMap::new();
    for line in text.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.splitn(3, '|');
        let (Some(_), Some(key), Some(hex)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let Some(bytes) = decode_hex(hex) else { continue };
        let acc = streams.entry(key.to_string()).or_insert_with(PacketAccumulator::new);
        acc.append(&bytes);
        let consumed = processor.consume_stream(acc.snapshot());
        if consumed > 0 {
            acc.discard_bytes(consumed);
        }
    }

    let names = storage.get_nicknames();
    for (healer, skills) in storage.get_heal_snapshot() {
        let (mut raw, mut effective, mut ticks, mut full, mut none) = (0i64, 0i64, 0usize, 0usize, 0usize);
        for data in skills.values() {
            assert_eq!(data.ticks.len(), data.effective.len());
            for (&(_, amount), &eff) in data.ticks.iter().zip(&data.effective) {
                assert!((0..=amount).contains(&eff), "{eff} of {amount}");
                raw += amount;
                effective += eff;
                ticks += 1;
                if eff == amount { full += 1; }
                if eff == 0 { none += 1; }
            }
        }
        let name = names.get(&healer).map(String::as_str).unwrap_or("?");
        println!(
            "{name} ({healer}): {ticks} ticks, reported {raw}, effective {effective} ({:.0} %), ticks fully effective {full}, all overheal {none}",
            100.0 * effective as f64 / raw.max(1) as f64
        );
    }
}
