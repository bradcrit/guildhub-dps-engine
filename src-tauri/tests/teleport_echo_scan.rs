//! Which frames repeat a map load's spot (`21 36`) in the seconds after it —
//! the self teleport, whatever its opcode is this week. For each map load:
//! every later frame (10 s) whose bytes hold the load's 12 spot bytes, with
//! its opcode, where the spot sits past the opcode, the delay and its length.
//!
//! Run with `A2_ECHO_CAPTURE=<packets_*.txt>[;<more>] cargo test --release --test teleport_echo_scan -- --nocapture`.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use parking_lot::Mutex;

use a2tools_dps_meter_lib::capture::frame_tap;
use a2tools_dps_meter_lib::capture::packet_accumulator::PacketAccumulator;
use a2tools_dps_meter_lib::capture::stream_processor::{read_varint, StreamProcessor};
use a2tools_dps_meter_lib::combat::data_storage::DataStorage;
use a2tools_dps_meter_lib::i18n::lookup::{NpcLookup, SkillLookup};
use a2tools_dps_meter_lib::share::read_capture;

#[derive(Default)]
struct Scan {
    loads: Vec<(i64, [u8; 12])>,
    /// load index → (delay ms, opcode, spot offset past the opcode, frame length)
    echoes: BTreeMap<usize, Vec<(i64, String, usize, usize, String)>>,
}

#[test]
fn teleport_echo() {
    let Ok(paths) = std::env::var("A2_ECHO_CAPTURE") else {
        eprintln!("A2_ECHO_CAPTURE unset; skipping");
        return;
    };
    for path in paths.split(';').filter(|p| !p.is_empty()) {
        scan(path);
    }
}

fn scan(path: &str) {
    let scan: Arc<Mutex<Scan>> = Arc::default();
    {
        let scan = scan.clone();
        frame_tap::set(Some(Arc::new(move |packet: &[u8]| {
            let len = read_varint(packet, 0);
            if len.length <= 0 {
                return;
            }
            let at = len.length as usize;
            let Some(op) = packet.get(at..at + 2) else { return };
            let now = a2tools_dps_meter_lib::clock::now_ms();
            let mut s = scan.lock();
            if op == [0x21, 0x36] {
                if let Some(b) = packet.get(at + 18..at + 30) {
                    s.loads.push((now, b.try_into().unwrap()));
                }
                return;
            }
            let body = &packet[at..];
            let hits: Vec<(usize, i64, usize)> = s
                .loads
                .iter()
                .enumerate()
                .filter(|(_, (t, _))| now - t <= 10_000)
                .filter_map(|(i, (t, spot))| body.windows(12).position(|w| w == spot).map(|k| (i, now - t, k)))
                .collect();
            for (i, dt, k) in hits {
                let hex: String = body.iter().take(40).map(|b| format!("{b:02x}")).collect();
                s.echoes.entry(i).or_default().push((dt, format!("{:02X} {:02X}", op[0], op[1]), k, packet.len(), hex));
            }
        })));
    }
    let mut processor = StreamProcessor::new(Arc::new(DataStorage::new()), Arc::new(SkillLookup::new()), Arc::new(NpcLookup::new()));
    let mut streams: HashMap<String, PacketAccumulator> = HashMap::new();
    for p in read_capture(std::path::Path::new(path)).unwrap() {
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

    let s = scan.lock();
    println!("\n== {path}: {} map loads", s.loads.len());
    for (i, _) in s.loads.iter().enumerate() {
        match s.echoes.get(&i) {
            None => println!("  load {i}: no echo"),
            Some(v) => {
                for (dt, op, k, len, hex) in v {
                    println!("  load {i}: +{dt:>5} ms  {op}  spot at +{k}  len {len}  {hex}");
                }
            }
        }
    }
}
