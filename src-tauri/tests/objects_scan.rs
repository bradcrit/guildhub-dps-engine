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
    last_gather: Option<i32>,
    unknown_at_gather: u32,
    /// At each spot told after a gap: how far the estimate before was off, its accuracy, guessed, the gap (s).
    checks: Vec<(f32, f32, bool, f32)>,
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
        // A2_OBJ_RADII=<sight_radii.json>: distances learnt before, as the engine keeps them.
        if let Some(radii) = std::env::var("A2_OBJ_RADII").ok().and_then(|f| std::fs::read_to_string(f).ok()).and_then(|t| serde_json::from_str(&t).ok()) {
            storage.set_sight_radii(radii);
        }
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
                        let fix = storage.self_fix(60_000);
                        if let Some(p) = pos {
                            let line = match fix {
                                Some(f) => {
                                    let off = f.pos.metres_to(&p);
                                    if s.last_gather != Some(id) { s.gathered.push((off, f.accuracy_m as i64)); }
                                    format!("engine {off:.0} m off, says ±{:.0} m{}{}", f.accuracy_m, if f.guessed { " (guess)" } else { "" }, f.age_ms.map_or(String::new(), |a| format!(", told {a} ms ago")))
                                }
                                None => { if s.last_gather != Some(id) { s.unknown_at_gather += 1; } "engine: no spot".into() }
                            };
                            eprintln!("{} you gather {id}: {line}", a2tools_dps_meter_lib::clock::now_ms() / 1000 % 1000);
                        }
                        s.last_gather = Some(id);
                    }
                    _ => {}
                }
            })));
        }
        let mut processor = StreamProcessor::new(storage.clone(), Arc::new(SkillLookup::new()), Arc::new(NpcLookup::new()));
        let mut acc = PacketAccumulator::new();
        let mut last_at: Option<i64> = None;
        let (mut secs, mut known, mut guessed, mut last_sec) = (0u32, 0u32, 0u32, 0i64);
        let mut accs: Vec<f32> = Vec::new();
        for p in read_capture(std::path::Path::new(path)).unwrap() {
            a2tools_dps_meter_lib::clock::set_override(Some(p.captured_at_ms));
            processor.set_override_timestamp(Some(p.captured_at_ms));
            let before = storage.self_fix(60_000);
            let told_before = storage.self_position().map(|(_, age)| p.captured_at_ms - age);
            acc.append(&p.bytes);
            let used = processor.consume_stream(acc.snapshot());
            if used > 0 { acc.discard_bytes(used); }
            if p.captured_at_ms / 1000 != last_sec {
                last_sec = p.captured_at_ms / 1000;
                secs += 1;
                match storage.self_fix(60_000) {
                    Some(f) if f.guessed || f.age_ms.is_some_and(|a| a <= 30_000) => { known += 1; if f.guessed { guessed += 1; } accs.push(f.accuracy_m); }
                    _ => {}
                }
            }
            if let (Some((told, age)), Some(prev)) = (storage.self_position(), told_before) {
                let told_at = p.captured_at_ms - age;
                if told_at != prev && told_at - prev >= 3_000 {
                    if let Some(f) = before {
                        let off = f.pos.metres_to(&told);
                        let stale = before.map(|_| ()).and(None::<()>);
                        let _ = stale;
                        let mut s = scan.lock();
                        s.checks.push((off, f.accuracy_m, f.guessed, (told_at - prev) as f32 / 1000.0));
                    }
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
        for (off, acc, g, gap) in &s.checks { eprintln!("  told after {gap:.0} s: estimate {off:.0} m off, said ±{acc:.0} m{}", if *g { " (guess)" } else { "" }); }
        let mut offs: Vec<f32> = s.checks.iter().map(|c| c.0).collect();
        histogram("estimate off at each spot told after a gap", &mut offs);
        eprintln!("  within its accuracy: {}/{}", s.checks.iter().filter(|c| c.0 <= c.1 + 1.0).count(), s.checks.len());
        if let Ok(out) = std::env::var("A2_OBJ_RADII_OUT") { if let Some((_, r)) = storage.sight_radii_since(0) { std::fs::write(out, serde_json::to_string(&r).unwrap()).unwrap(); } }
        eprintln!("seconds with a spot shown (as the gather map would): {known}/{secs}, of them guessed {guessed}");
        histogram("  its accuracy each second (m)", &mut accs);
        eprintln!("  your entity: {:?}", storage.local_player_id());
        histogram("come into range", &mut s.come);
        histogram("go out of range", &mut s.gone);
        let mut gaps: Vec<f32> = s.fix_gaps.iter().map(|g| *g as f32 / 1000.0).collect();
        eprintln!("spots told: {}", s.fixes);
        histogram("gaps between spots told (s, not m)", &mut gaps);
        let mut g: Vec<f32> = s.gathered.iter().map(|(d, _)| *d).collect();
        histogram("before each gather: engine metres off", &mut g);
        let inside = s.gathered.iter().filter(|(d, acc)| *d <= *acc as f32 + 1.0).count();
        eprintln!("  within the accuracy it said: {inside}/{}, no spot at all: {}", s.gathered.len(), s.unknown_at_gather);
    }
}
