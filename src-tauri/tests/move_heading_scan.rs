//! What a move record (`1A/1B/1C/3C 37`) carries besides the spot — looking
//! for a heading: prints one entity's move frames as hex with the spot
//! marked, and the floats after it, next to the direction it moved.
//!
//! Run with `A2_HEADING_CAPTURE=<packets_*.txt> A2_HEADING_ENTITY=<id> cargo test --release --test move_heading_scan -- --nocapture`.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use a2tools_dps_meter_lib::capture::frame_tap;
use a2tools_dps_meter_lib::capture::packet_accumulator::PacketAccumulator;
use a2tools_dps_meter_lib::capture::positions::{self, Spotted};
use a2tools_dps_meter_lib::capture::stream_processor::{read_varint, StreamProcessor};
use a2tools_dps_meter_lib::combat::data_storage::DataStorage;
use a2tools_dps_meter_lib::i18n::lookup::{NpcLookup, SkillLookup};
use a2tools_dps_meter_lib::share::read_capture;

#[test]
fn move_heading() {
    let (Ok(path), Ok(entity)) = (std::env::var("A2_HEADING_CAPTURE"), std::env::var("A2_HEADING_ENTITY")) else {
        eprintln!("A2_HEADING_CAPTURE / A2_HEADING_ENTITY unset; skipping");
        return;
    };
    let entity: i32 = entity.parse().unwrap();
    let storage = Arc::new(DataStorage::new());
    let rows: Arc<Mutex<Vec<String>>> = Arc::default();
    let last: Arc<Mutex<Option<(f32, f32)>>> = Arc::default();
    {
        let rows = rows.clone();
        let last = last.clone();
        frame_tap::set(Some(Arc::new(move |packet: &[u8]| {
            let len = read_varint(packet, 0);
            if len.length <= 0 {
                return;
            }
            let at = len.length as usize;
            let Some(op) = packet.get(at..at + 2) else { return };
            if !matches!(op, [0x1A | 0x1B | 0x1C | 0x3C, 0x37]) {
                return;
            }
            let id = read_varint(packet, at + 2);
            if id.value != entity {
                return;
            }
            let Some(Spotted::Entity(_, pos)) = positions::parse(packet) else { return };
            let body = &packet[at..];
            let hex: Vec<String> = body.iter().map(|b| format!("{b:02x}")).collect();
            let after = at + 2 + id.length as usize;
            // Floats from the spot on.
            let floats: Vec<String> = (0..10)
                .filter_map(|k| packet.get(after + 1 + 4 * k..after + 5 + 4 * k).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])))
                .map(|f| format!("{f:.3}"))
                .collect();
            let mut last = last.lock();
            let dir = last.map(|(x, y)| ((pos.y - y).atan2(pos.x - x)).to_degrees());
            *last = Some((pos.x, pos.y));
            let now = a2tools_dps_meter_lib::clock::now_ms();
            rows.lock().push(format!(
                "{now} {:02X} len {} move-dir {:>7} | floats {} | {}",
                op[0],
                body.len(),
                dir.map_or("-".into(), |d| format!("{d:.0}")),
                floats.join(" "),
                hex.join(" ")
            ));
        })));
    }
    let mut processor = StreamProcessor::new(storage.clone(), Arc::new(SkillLookup::new()), Arc::new(NpcLookup::new()));
    let mut streams: HashMap<String, PacketAccumulator> = HashMap::new();
    for p in read_capture(std::path::Path::new(&path)).unwrap() {
        a2tools_dps_meter_lib::clock::set_override(Some(p.captured_at_ms));
        processor.set_override_timestamp(Some(p.captured_at_ms));
        let acc = streams.entry(p.stream.clone()).or_insert_with(PacketAccumulator::new);
        acc.append(&p.bytes);
        let used = processor.consume_stream(acc.snapshot());
        if used > 0 {
            acc.discard_bytes(used);
        }
    }
    frame_tap::set(None);
    a2tools_dps_meter_lib::clock::set_override(None);
    for r in rows.lock().iter().take(60) {
        eprintln!("{r}");
    }
    eprintln!("{} move frames", rows.lock().len());
}
