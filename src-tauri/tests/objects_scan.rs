//! How far objects (gathering spots …) come into and go out of range from
//! you, and how close you stand to the ones you gather — whether they can
//! tell your spot. Run with `A2_OBJ_CAPTURE=<packets_*.txt> cargo test
//! --release --test objects_scan -- --nocapture` (several: `;` between).
//! A2_OBJ_ME=<entity>: your entity id, for a capture started after login
//! (the actor of your `38 36`). Objects already in range when the capture
//! started have no spot. 2026-11-07 22:20: gathering told your spot to the
//! metre where it had been 25 m off before.

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use a2tools_dps_meter_lib::capture::frame_tap;
use a2tools_dps_meter_lib::capture::objects::{self, ObjectRecord};
use a2tools_dps_meter_lib::capture::packet_accumulator::PacketAccumulator;
use a2tools_dps_meter_lib::capture::positions::Position;
use a2tools_dps_meter_lib::capture::stream_processor::StreamProcessor;
use a2tools_dps_meter_lib::combat::data_storage::DataStorage;
use a2tools_dps_meter_lib::i18n::lookup::{NpcLookup, SkillLookup};
use a2tools_dps_meter_lib::share::read_capture;

const FRESH_MS: i64 = 1_500;

#[derive(Default)]
struct Scan {
    objects: HashMap<i32, (u32, Position)>,
    come: Vec<f32>,
    gone: Vec<f32>,
    gathered: Vec<(f32, i64)>,
    fixes: u32,
    fix_gaps: Vec<i64>,
}

fn histogram(name: &str, values: &mut [f32]) {
    values.sort_by(f32::total_cmp);
    let at = |q: f32| values.get(((values.len() as f32 - 1.0) * q) as usize).copied().unwrap_or(f32::NAN);
    eprintln!("{name}: {} — p10 {:.0}, p25 {:.0}, p50 {:.0}, p75 {:.0}, p90 {:.0} m", values.len(), at(0.1), at(0.25), at(0.5), at(0.75), at(0.9));
}

#[test]
fn scan_objects() {
    let Ok(paths) = std::env::var("A2_OBJ_CAPTURE") else { return };
    for path in paths.split(';') {
        let storage = Arc::new(DataStorage::new());
        if let Some(me) = std::env::var("A2_OBJ_ME").ok().and_then(|v| v.parse().ok()) { storage.set_local_player_id(Some(me)); }
        let scan = Arc::new(Mutex::new(Scan::default()));
        {
            let storage = storage.clone();
            let scan = scan.clone();
            frame_tap::set(Some(Arc::new(move |packet: &[u8]| {
                let Some(record) = objects::parse(packet) else { return };
                let fresh = storage.self_position().filter(|(_, age)| *age <= FRESH_MS).map(|(p, _)| p);
                let me = storage.local_player_id();
                let mut s = scan.lock();
                match record {
                    ObjectRecord::InRange(id, code, pos, _) => {
                        eprintln!("{} in range: {id} code {code} ({:.0}, {:.0}){}", a2tools_dps_meter_lib::clock::now_ms() / 1000 % 1000, pos.x / 100.0, pos.y / 100.0,
                            storage.self_estimate(60_000).map_or(String::new(), |(e, _, g)| format!(", {:.0} m from your {}", e.metres_to(&pos), if g { "guess" } else { "spot" })));
                        if let Some(f) = fresh { s.come.push(f.metres_to(&pos)); }
                        s.objects.insert(id, (code, pos));
                    }
                    ObjectRecord::Gone(id, used) => {
                        eprintln!("{} gone: {id} used {used}{}", a2tools_dps_meter_lib::clock::now_ms() / 1000 % 1000,
                            match (s.objects.get(&id), storage.self_estimate(60_000)) { (Some((_, p)), Some((e, _, g))) => format!(", {:.0} m from your {}", e.metres_to(p), if g { "guess" } else { "spot" }), _ => String::new() });
                        if let Some((_, pos)) = s.objects.remove(&id) {
                            if !used { if let Some(f) = fresh { s.gone.push(f.metres_to(&pos)); } }
                        }
                    }
                    ObjectRecord::Opening(id, actor) if me == Some(actor as i64) => {
                        let pos = s.objects.get(&id).map(|o| o.1);
                        let told = storage.self_position();
                        let est = storage.self_estimate(60_000);
                        let est = match (pos, est) { (Some(p), Some((e, _, g))) => format!("engine {:.0} m off{}", e.metres_to(&p), if g { " (guess)" } else { "" }), _ => "engine: none".into() };
                        eprintln!("you gather {id} (code {:?}), {est}: {}", s.objects.get(&id).map(|o| o.0),
                            match (pos, told) { (Some(p), Some((t, age))) => format!("{:.1} m from your spot told {age} ms ago", t.metres_to(&p)), _ => "no spot".into() });
                        if let (Some(p), Some((t, age))) = (pos, told) { s.gathered.push((t.metres_to(&p), age)); }
                    }
                    _ => {}
                }
            })));
        }
        let mut processor = StreamProcessor::new(storage.clone(), Arc::new(SkillLookup::new()), Arc::new(NpcLookup::new()));
        let mut acc = PacketAccumulator::new();
        let mut last_at: Option<i64> = None;
        let (mut secs, mut known, mut guessed, mut last_sec) = (0u32, 0u32, 0u32, 0i64);
        for p in read_capture(std::path::Path::new(path)).unwrap() {
            a2tools_dps_meter_lib::clock::set_override(Some(p.captured_at_ms));
            processor.set_override_timestamp(Some(p.captured_at_ms));
            acc.append(&p.bytes);
            let used = processor.consume_stream(acc.snapshot());
            if used > 0 { acc.discard_bytes(used); }
            if p.captured_at_ms / 1000 != last_sec {
                last_sec = p.captured_at_ms / 1000;
                secs += 1;
                match storage.self_estimate(60_000) {
                    Some((_, age, g)) if g || age.is_some_and(|a| a <= 30_000) => { known += 1; if g { guessed += 1; } }
                    _ => {}
                }
            }
            if let Some((_, age)) = storage.self_position() {
                let told = p.captured_at_ms - age;
                if last_at != Some(told) {
                    let mut s = scan.lock();
                    if let Some(prev) = last_at { s.fix_gaps.push(told - prev); }
                    s.fixes += 1;
                    last_at = Some(told);
                }
            }
        }
        frame_tap::set(None);
        a2tools_dps_meter_lib::clock::set_override(None);
        let mut s = scan.lock();
        eprintln!("== {path}");
        eprintln!("seconds with a spot shown (as the gather map would): {known}/{secs}, of them guessed {guessed}");
        histogram("come into range", &mut s.come);
        histogram("go out of range", &mut s.gone);
        let mut gaps: Vec<f32> = s.fix_gaps.iter().map(|g| *g as f32 / 1000.0).collect();
        eprintln!("spots told: {}", s.fixes);
        histogram("gaps between spots told (s, not m)", &mut gaps);
        let mut g: Vec<f32> = s.gathered.iter().filter(|(_, age)| *age <= 3_000).map(|(d, _)| *d).collect();
        histogram("you gathering: metres from a spot told ≤3 s before", &mut g);
    }
}
