//! Replays a packet capture at its own times and prints what the party frames
//! (`gh_engine`'s `party` event) are made of, every 30 s: each roster member's
//! entity, HP and MP (`1b 92`) and the debuffs on them, and at the end the
//! debuffs another player's skill took off (what `learn_cleansable` learns).
//!
//! What it checks: an HP or MP read is never above its maximum, and a party
//! member's entity is the one `1b 92` named last.
//!
//!   A2_PARTY_CAPTURE=<capture> cargo test --test party_frames_replay -- --nocapture

use std::collections::HashMap;
use std::sync::Arc;

use a2tools_dps_meter_lib::capture::packet_accumulator::PacketAccumulator;
use a2tools_dps_meter_lib::capture::resources::{HP, MP};
use a2tools_dps_meter_lib::capture::stream_processor::StreamProcessor;
use a2tools_dps_meter_lib::clock;
use a2tools_dps_meter_lib::combat::data_storage::DataStorage;
use a2tools_dps_meter_lib::i18n::lookup::{NpcLookup, SkillLookup};

fn decode_hex(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok()).collect()
}

fn print_party(storage: &DataStorage, at_ms: i64) {
    let mut roster: Vec<_> = storage.get_party_members().into_iter().collect();
    if roster.len() < 2 {
        return;
    }
    roster.sort_by_key(|(name, m)| (m.slot, name.clone()));
    let time = chrono::DateTime::from_timestamp_millis(at_ms).unwrap().with_timezone(&chrono::Local).format("%H:%M:%S");
    println!("{time}");
    for (name, member) in roster {
        let id = storage.party_member_id(&name);
        let resources = id.map(|id| storage.entity_resources(id)).unwrap_or_default();
        let (hp, mp) = (resources.wide(HP), resources.small(MP));
        for g in [hp, mp].into_iter().flatten() {
            assert!(g.current <= g.max, "{name}: {} of {}", g.current, g.max);
        }
        let debuffs: Vec<_> = id
            .map(|id| storage.abnormals_on(id))
            .unwrap_or_default()
            .into_iter()
            .filter(|(i, end)| end.is_none_or(|e| e > at_ms) && storage.is_hostile_caster(i.caster))
            .map(|(i, _)| i.abnormal)
            .collect();
        println!(
            "  {} {name} ({id:?}): hp {:?} mp {:?} hostile {debuffs:?}",
            member.slot,
            hp.map(|g| (g.current, g.max)),
            mp.map(|g| (g.current, g.max)),
        );
    }
}

#[test]
fn a_capture_feeds_the_party_frames() {
    let Ok(path) = std::env::var("A2_PARTY_CAPTURE") else {
        eprintln!("A2_PARTY_CAPTURE unset — skipping");
        return;
    };
    let storage = Arc::new(DataStorage::new());
    let mut processor = StreamProcessor::new(storage.clone(), Arc::new(SkillLookup::new()), Arc::new(NpcLookup::new()));
    let text = std::fs::read_to_string(&path).expect("capture readable");
    let mut streams: HashMap<String, PacketAccumulator> = HashMap::new();
    let mut taken_off: HashMap<(u32, bool), usize> = HashMap::new();
    let mut printed_ms = 0;
    for line in text.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.splitn(3, '|');
        let (Some(at), Some(key), Some(hex)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let Ok(at) = chrono::DateTime::parse_from_rfc3339(at) else { continue };
        let Some(bytes) = decode_hex(hex) else { continue };
        let at_ms = at.timestamp_millis();
        clock::set_override(Some(at_ms));
        let acc = streams.entry(key.to_string()).or_insert_with(PacketAccumulator::new);
        acc.append(&bytes);
        let consumed = processor.consume_stream(acc.snapshot());
        if consumed > 0 {
            acc.discard_bytes(consumed);
        }
        for (abnormal, entity, by) in storage.take_abnormals_taken_off() {
            if by != entity && storage.is_player(by) && storage.is_player(entity) {
                *taken_off.entry((abnormal, true)).or_default() += 1;
            } else {
                *taken_off.entry((abnormal, false)).or_default() += 1;
            }
        }
        if at_ms - printed_ms >= 30_000 {
            printed_ms = at_ms;
            print_party(&storage, at_ms);
        }
    }
    clock::set_override(None);

    let mut taken_off: Vec<_> = taken_off.into_iter().collect();
    taken_off.sort_by_key(|&((abnormal, by_other), n)| (!by_other, std::cmp::Reverse(n), abnormal));
    println!("taken off (abnormal, by another player): times");
    for ((abnormal, by_other), n) in taken_off {
        println!("  {abnormal} {by_other}: {n}");
    }
}
