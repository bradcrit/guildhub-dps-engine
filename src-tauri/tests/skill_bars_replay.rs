//! Replays a packet capture at its own times and prints what the cooldown and
//! cast bars (`combat::skill_timers`) made of it: the monster cast times
//! learned, and your cooldowns and buffs once the capture is over.
//!
//! What it checks: a learned cast time is within the 6 s a hit may follow its
//! cast, and a cooldown ends after the capture's last packet only when it
//! started within its length of it.
//!
//!   A2_SKILLS_CAPTURE=<capture> cargo test --test skill_bars_replay -- --nocapture

use std::collections::HashMap;
use std::sync::Arc;

use a2tools_dps_meter_lib::capture::packet_accumulator::PacketAccumulator;
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

#[test]
fn a_capture_feeds_the_skill_bars() {
    let Ok(path) = std::env::var("A2_SKILLS_CAPTURE") else {
        eprintln!("A2_SKILLS_CAPTURE unset — skipping");
        return;
    };
    let storage = Arc::new(DataStorage::new());
    let mut processor = StreamProcessor::new(storage.clone(), Arc::new(SkillLookup::new()), Arc::new(NpcLookup::new()));
    let text = std::fs::read_to_string(&path).expect("capture readable");
    let mut streams: HashMap<String, PacketAccumulator> = HashMap::new();
    let mut last_ms = 0;
    for line in text.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.splitn(3, '|');
        let (Some(at), Some(key), Some(hex)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let Ok(at) = chrono::DateTime::parse_from_rfc3339(at) else { continue };
        let Some(bytes) = decode_hex(hex) else { continue };
        last_ms = at.timestamp_millis();
        clock::set_override(Some(last_ms));
        let acc = streams.entry(key.to_string()).or_insert_with(PacketAccumulator::new);
        acc.append(&bytes);
        let consumed = processor.consume_stream(acc.snapshot());
        if consumed > 0 {
            acc.discard_bytes(consumed);
        }
    }

    let (_, learned) = storage.cast_times_since(0).unwrap_or_default();
    let mut learned: Vec<_> = learned.into_iter().collect();
    learned.sort();
    for (skill, samples) in &learned {
        assert!(samples.iter().all(|&ms| ms <= 6_000), "{skill}: {samples:?}");
        println!("monster skill {skill}: cast {samples:?} ms");
    }
    for c in storage.skill_cooldowns() {
        assert!(c.ends_ms > last_ms && c.ends_ms - last_ms <= i64::from(c.total_ms));
        println!("cooldown {}: {} ms of {} left", c.skill, c.ends_ms - last_ms, c.total_ms);
    }
    for (buff, end) in storage.self_abnormals() {
        println!("on you: abnormal {} (skill {:?}), ends {:?}", buff.abnormal, buff.skill, end.map(|e| e - last_ms));
    }
    clock::set_override(None);
}
