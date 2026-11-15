//! Are the distances things come into sight and leave it at fixed, per code
//! (as a game server's cull distance per class would be), once the noise of
//! the measurement is taken out? Run with
//! `A2_RADII_CAPTURE=<packets_*.txt>[;…] cargo test --release --test sight_radii_scan -- --nocapture`.
//!
//! For every mob coming into sight (`41 36`) or leaving it (`42 36` flag 1),
//! player leaving (`47 36`) and object coming into range / going out of it,
//! it notes the distance from your spot told — on the ground (as the engine
//! learns it) and in 3D — with how old your spot and its spot were. Then,
//! per code, how widely those distances spread when both spots were fresh.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use parking_lot::Mutex;

use a2tools_dps_meter_lib::capture::frame_tap;
use a2tools_dps_meter_lib::capture::objects::{self, ObjectRecord};
use a2tools_dps_meter_lib::capture::packet_accumulator::PacketAccumulator;
use a2tools_dps_meter_lib::capture::positions::{self, Position, Spotted};
use a2tools_dps_meter_lib::capture::stream_processor::{read_varint, StreamProcessor};
use a2tools_dps_meter_lib::combat::data_storage::DataStorage;
use a2tools_dps_meter_lib::i18n::lookup::{NpcLookup, SkillLookup};
use a2tools_dps_meter_lib::share::read_capture;

/// Your spot counts as fresh while this young (ms) — running 5 m/s, ±2.5 m.
const FRESH_MS: i64 = 500;
/// A mob's spot counts as fresh while this young (ms).
const ENTITY_FRESH_MS: i64 = 1_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    MobIn,
    MobOut,
    PlayerOut,
    ObjectIn,
    ObjectOut,
}

#[derive(Clone, Copy)]
struct Event {
    kind: Kind,
    /// Entity id (mobs: the code is looked up at the end).
    id: i32,
    /// Object code, where known at once.
    code: Option<u32>,
    d2: f32,
    d3: f32,
    /// The larger of |dx|, |dy| (m): a square grid's distance.
    cheb: f32,
    fix_age: i64,
    spot_age: i64,
}

#[derive(Default)]
struct Scan {
    spots: HashMap<i32, (Position, i64)>,
    objects: HashMap<i32, (u32, Position)>,
    events: Vec<Event>,
}

fn d3(a: &Position, b: &Position) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2) + (a.z - b.z).powi(2)).sqrt() / 100.0
}

fn q(v: &[f32], at: f32) -> f32 {
    v[((v.len() - 1) as f32 * at).round() as usize]
}

#[test]
fn scan_sight_radii() {
    let Ok(paths) = std::env::var("A2_RADII_CAPTURE") else { return };
    let mut all: Vec<(Event, Option<i32>)> = Vec::new();
    for path in paths.split(';') {
        let storage = Arc::new(DataStorage::new());
        let scan = Arc::new(Mutex::new(Scan::default()));
        {
            let storage = storage.clone();
            let scan = scan.clone();
            frame_tap::set(Some(Arc::new(move |packet: &[u8]| {
                let now = a2tools_dps_meter_lib::clock::now_ms();
                let me = storage.self_position();
                let mut s = scan.lock();
                let push = |s: &mut Scan, kind, id, code, spot: Position, spot_at: i64| {
                    if let Some((p, age)) = me {
                        s.events.push(Event { kind, id, code, d2: p.metres_to(&spot), d3: d3(&p, &spot), cheb: (p.x - spot.x).abs().max((p.y - spot.y).abs()) / 100.0, fix_age: age, spot_age: now - spot_at });
                    }
                };
                if let Some(record) = objects::parse(packet) {
                    match record {
                        ObjectRecord::InRange(id, code, pos, created) => {
                            if created.is_none_or(|at| now - at > 5_000) {
                                push(&mut s, Kind::ObjectIn, id, Some(code), pos, now);
                            }
                            s.objects.insert(id, (code, pos));
                        }
                        ObjectRecord::Gone(id, used) => {
                            if let Some((code, pos)) = s.objects.remove(&id) {
                                if !used {
                                    push(&mut s, Kind::ObjectOut, id, Some(code), pos, now);
                                }
                            }
                        }
                        _ => {}
                    }
                }
                let len = read_varint(packet, 0);
                if len.length <= 0 {
                    return;
                }
                let at = len.length as usize;
                let Some(op) = packet.get(at..at + 2) else { return };
                match positions::parse(packet) {
                    Some(Spotted::You(_)) => s.spots.clear(),
                    Some(Spotted::Entity(id, pos)) => {
                        if op == [0x41, 0x36] {
                            push(&mut s, Kind::MobIn, id, None, pos, now);
                        }
                        s.spots.insert(id, (pos, now));
                    }
                    _ => {}
                }
                let entity = read_varint(packet, at + 2);
                if entity.length <= 0 {
                    return;
                }
                let id = entity.value;
                if op == [0x42, 0x36] {
                    let mut i = at + 2 + entity.length as usize;
                    let skip = read_varint(packet, i);
                    if skip.length <= 0 {
                        return;
                    }
                    i += skip.length as usize;
                    let flag = read_varint(packet, i);
                    if flag.length > 0 && flag.value == 1 {
                        if let Some((pos, t)) = s.spots.remove(&id) {
                            push(&mut s, Kind::MobOut, id, None, pos, t);
                        }
                    }
                } else if op == [0x47, 0x36] {
                    if let Some((pos, t)) = s.spots.remove(&id) {
                        push(&mut s, Kind::PlayerOut, id, None, pos, t);
                    }
                }
            })));
        }
        let mut processor = StreamProcessor::new(storage.clone(), Arc::new(SkillLookup::new()), Arc::new(NpcLookup::new()));
        let mut acc = PacketAccumulator::new();
        for p in read_capture(std::path::Path::new(path)).unwrap() {
            a2tools_dps_meter_lib::clock::set_override(Some(p.captured_at_ms));
            processor.set_override_timestamp(Some(p.captured_at_ms));
            acc.append(&p.bytes);
            let used = processor.consume_stream(acc.snapshot());
            if used > 0 {
                acc.discard_bytes(used);
            }
        }
        frame_tap::set(None);
        let events = std::mem::take(&mut scan.lock().events);
        for e in events {
            let code = match e.kind {
                Kind::MobIn | Kind::MobOut => storage.mob_code(e.id),
                Kind::ObjectIn | Kind::ObjectOut => e.code.map(|c| c as i32),
                Kind::PlayerOut => (storage.mob_code(e.id).is_none()).then_some(0),
            };
            all.push((e, code));
        }
    }

    eprintln!("events: {}", all.len());
    // Per kind and code, fresh only: spread of the 2D and 3D distances.
    let mut groups: BTreeMap<(Kind, i32), Vec<(f32, f32)>> = BTreeMap::new();
    let mut stale: BTreeMap<Kind, (u32, u32)> = BTreeMap::new();
    for (e, code) in &all {
        let Some(code) = code else { continue };
        let fresh = e.fix_age <= FRESH_MS && e.spot_age <= ENTITY_FRESH_MS && e.d2 < 300.0;
        let c = stale.entry(e.kind).or_default();
        if fresh {
            c.0 += 1;
            groups.entry((e.kind, *code)).or_default().push((e.d2, e.d3));
        } else {
            c.1 += 1;
        }
    }
    eprintln!("fresh / not fresh by kind: {stale:?}");
    // All codes together, 2 m buckets: a circle (round) against a square grid (cheb).
    for kind in [Kind::MobIn, Kind::MobOut, Kind::PlayerOut, Kind::ObjectIn, Kind::ObjectOut] {
        let pick: Vec<&Event> = all
            .iter()
            .map(|(e, _)| e)
            .filter(|e| e.kind == kind && e.fix_age <= FRESH_MS && e.spot_age <= ENTITY_FRESH_MS && e.d2 < 300.0 && (kind != Kind::MobIn || e.d2 > 15.0))
            .collect();
        for (name, f) in [("round", (|e: &Event| e.d2) as fn(&Event) -> f32), ("cheb ", |e: &Event| e.cheb)] {
            let mut b: BTreeMap<u32, u32> = BTreeMap::new();
            for e in &pick {
                *b.entry((f(e) / 2.0) as u32 * 2).or_default() += 1;
            }
            eprintln!("{kind:?} {name} n {}: {}", pick.len(), b.iter().map(|(k, n)| format!("{k}:{n}")).collect::<Vec<_>>().join(" "));
        }
    }
    let mut rel2_all: BTreeMap<Kind, Vec<f32>> = BTreeMap::new();
    let mut rel3_all: BTreeMap<Kind, Vec<f32>> = BTreeMap::new();
    for ((kind, code), v) in &groups {
        if v.len() < 4 {
            continue;
        }
        let mut a: Vec<f32> = v.iter().map(|x| x.0).collect();
        let mut b: Vec<f32> = v.iter().map(|x| x.1).collect();
        a.sort_by(f32::total_cmp);
        b.sort_by(f32::total_cmp);
        let (m2, m3) = (q(&a, 0.5), q(&b, 0.5));
        let (iqr2, iqr3) = (q(&a, 0.75) - q(&a, 0.25), q(&b, 0.75) - q(&b, 0.25));
        rel2_all.entry(*kind).or_default().push(iqr2);
        rel3_all.entry(*kind).or_default().push(iqr3);
        eprintln!(
            "{kind:?} {code:>11}: n {:>3}  2D p10 {:>5.1} p50 {:>5.1} p90 {:>5.1} IQR {:>4.1}  |  3D p10 {:>5.1} p50 {:>5.1} p90 {:>5.1} IQR {:>4.1}  max {:>5.1}",
            v.len(),
            q(&a, 0.1),
            m2,
            q(&a, 0.9),
            iqr2,
            q(&b, 0.1),
            m3,
            q(&b, 0.9),
            iqr3,
            b[b.len() - 1]
        );
    }
    for (kind, v) in rel2_all.iter_mut() {
        v.sort_by(f32::total_cmp);
        let w = rel3_all.get_mut(kind).unwrap();
        w.sort_by(f32::total_cmp);
        eprintln!("{kind:?}: codes {} — median IQR 2D {:.1} m, 3D {:.1} m", v.len(), q(v, 0.5), q(w, 0.5));
    }
}
