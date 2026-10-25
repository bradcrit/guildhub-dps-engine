//! Replays a packet capture and prints what the resource bars
//! (`capture::resources`) made of it: your HP, MP, stamina and flight time
//! whenever one changed, and how often.
//!
//! What it checks: once its maximum is known, a resource never reads above
//! it, and your stamina and flight time (if they changed) were read.
//!
//!   A2_RESOURCES_CAPTURE=<capture> cargo test --test resource_bars_replay -- --nocapture

use std::collections::HashMap;
use std::sync::Arc;

use a2tools_dps_meter_lib::capture::packet_accumulator::PacketAccumulator;
use a2tools_dps_meter_lib::capture::resources::{Gauge, FLIGHT, HP, MP, STAMINA};
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
fn a_capture_feeds_the_resource_bars() {
    let Ok(path) = std::env::var("A2_RESOURCES_CAPTURE") else {
        eprintln!("A2_RESOURCES_CAPTURE unset — skipping");
        return;
    };
    let storage = Arc::new(DataStorage::new());
    let mut processor = StreamProcessor::new(storage.clone(), Arc::new(SkillLookup::new()), Arc::new(NpcLookup::new()));
    let text = std::fs::read_to_string(&path).expect("capture readable");
    let mut streams: HashMap<String, PacketAccumulator> = HashMap::new();
    let mut last: [Option<Gauge>; 4] = [None; 4];
    let mut changes = [0usize; 4];
    for line in text.lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.splitn(3, '|');
        let (Some(at), Some(key), Some(hex)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let Ok(at) = chrono::DateTime::parse_from_rfc3339(at) else { continue };
        let Some(bytes) = decode_hex(hex) else { continue };
        clock::set_override(Some(at.timestamp_millis()));
        let acc = streams.entry(key.to_string()).or_insert_with(PacketAccumulator::new);
        acc.append(&bytes);
        let consumed = processor.consume_stream(acc.snapshot());
        if consumed > 0 {
            acc.discard_bytes(consumed);
        }

        let r = storage.self_resources();
        let now = [r.wide(HP), r.small(MP), r.small(STAMINA), r.small(FLIGHT)];
        for (i, gauge) in now.into_iter().enumerate() {
            if gauge != last[i] {
                if let Some(g) = gauge {
                    assert!(g.current <= g.max, "{g:?}");
                    if changes[i] < 5 {
                        println!("{} {}: {} / {}", at.format("%H:%M:%S%.3f"), ["hp", "mp", "stamina", "flight"][i], g.current, g.max);
                    }
                }
                changes[i] += 1;
                last[i] = gauge;
            }
        }
    }
    println!("changes: hp {}, mp {}, stamina {}, flight {}", changes[0], changes[1], changes[2], changes[3]);
    clock::set_override(None);
}
