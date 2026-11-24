//! What a spawn record (`41 36`) carries after the spot — looking for a
//! heading, as the move records have (`capture::positions::heading`): prints
//! the spawns of the given entities as hex from the spot on, with the `u16`
//! right after it read as degrees.
//!
//! Run with `A2_SPAWN_CAPTURE=<packets_*.txt> A2_SPAWN_ENTITIES=<id,id,…> cargo test --release --test spawn_tail_scan -- --nocapture`.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use parking_lot::Mutex;

use a2tools_dps_meter_lib::capture::frame_tap;
use a2tools_dps_meter_lib::capture::packet_accumulator::PacketAccumulator;
use a2tools_dps_meter_lib::capture::stream_processor::{read_varint, StreamProcessor};
use a2tools_dps_meter_lib::combat::data_storage::DataStorage;
use a2tools_dps_meter_lib::i18n::lookup::{NpcLookup, SkillLookup};
use a2tools_dps_meter_lib::share::read_capture;

#[test]
fn spawn_tail() {
    let (Ok(path), Ok(ids)) = (std::env::var("A2_SPAWN_CAPTURE"), std::env::var("A2_SPAWN_ENTITIES")) else {
        eprintln!("A2_SPAWN_CAPTURE / A2_SPAWN_ENTITIES unset; skipping");
        return;
    };
    let wanted: HashSet<i32> = ids.split(',').filter_map(|s| s.trim().parse().ok()).collect();
    let storage = Arc::new(DataStorage::new());
    let rows: Arc<Mutex<Vec<String>>> = Arc::default();
    {
        let rows = rows.clone();
        frame_tap::set(Some(Arc::new(move |packet: &[u8]| {
            let len = read_varint(packet, 0);
            if len.length <= 0 {
                return;
            }
            let at = len.length as usize;
            if packet.get(at..at + 2) != Some(&[0x41, 0x36]) {
                return;
            }
            let id = read_varint(packet, at + 2);
            if !wanted.contains(&id.value) {
                return;
            }
            let after = at + 2 + id.length as usize;
            let spot = after + 9;
            let f = |k: usize| packet.get(spot + 4 * k..spot + 4 * k + 4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]));
            let tail = packet.get(spot + 12..).unwrap_or(&[]);
            let u16s: Vec<String> = tail
                .chunks(2)
                .take(6)
                .filter(|c| c.len() == 2)
                .map(|c| format!("{:.0}", u16::from_le_bytes([c[0], c[1]]) as f32 * 360.0 / 65_536.0))
                .collect();
            let now = a2tools_dps_meter_lib::clock::now_ms();
            rows.lock().push(format!(
                "{now} id {} spot ({:.0}, {:.0}) | tail u16 as deg {} | {}",
                id.value,
                f(0).unwrap_or(f32::NAN) / 100.0,
                f(1).unwrap_or(f32::NAN) / 100.0,
                u16s.join(" "),
                tail.iter().take(24).map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
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
    for r in rows.lock().iter() {
        eprintln!("{r}");
    }
}
