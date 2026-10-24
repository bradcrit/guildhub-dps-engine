//! Looks through a real capture for spots the parser doesn't read yet, and
//! measures what your spot could be told from besides (`capture::positions`).
//!
//! Run with `A2_SCAN_CAPTURE=<packets_*.txt> cargo test --release --test positions_scan -- --nocapture`.
//! Prints:
//! - every frame type not read for spots that holds three floats where you
//!   could have been then (between the spots told before and after, at
//!   running speed), by offset â€” counted only in gaps of GAP_MS and more;
//! - how far from you entities come into sight and leave it;
//! - how far the middle of everything in sight is from you.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use parking_lot::Mutex;

use a2tools_dps_meter_lib::capture::frame_tap;
use a2tools_dps_meter_lib::capture::packet_accumulator::PacketAccumulator;
use a2tools_dps_meter_lib::capture::positions::{self, Position, Spotted};
use a2tools_dps_meter_lib::capture::stream_processor::{read_varint, StreamProcessor};
use a2tools_dps_meter_lib::combat::data_storage::DataStorage;
use a2tools_dps_meter_lib::i18n::lookup::{NpcLookup, SkillLookup};
use a2tools_dps_meter_lib::share::read_capture;

const FRESH_MS: i64 = 1_500;
/// A gap between two spots of yours this long and more is looked into.
const GAP_MS: i64 = 3_000;
/// A mount at most (304 m in ~12 s seen), in cm/ms.
const SPEED: f32 = 2.5;
/// An entity's spot counts as in sight while this young (centre of sight).
const SEEN_MS: i64 = 10_000;

#[derive(Default)]
struct Scan {
    /// (time, type, offset, whether the entity was you, spot).
    candidates: Vec<(i64, String, usize, bool, Position)>,
    types: HashMap<String, u32>,
    /// Your spots as told, in order.
    fixes: Vec<(i64, Position)>,
    seen: HashMap<i32, (i64, Position)>,
    appear: BTreeMap<String, Vec<f32>>,
    leave: Vec<f32>,
    /// What's in sight (by the parser's spots, until it despawns or you're put somewhere).
    sight: HashMap<i32, (Position, i64)>,
    /// At each spot of yours told: when, the spot, what was in sight (and when its spot was told).
    at_fixes: Vec<(i64, Position, Vec<(Position, i64)>)>,
    /// Each entity in sight: the last frame type that named it, and when.
    last_kind: HashMap<i32, (String, i64)>,
    /// `42 36` by its flag: how far the entity was from you.
    gone: BTreeMap<u8, Vec<f32>>,
}

fn floats(packet: &[u8], i: usize) -> Option<Position> {
    let f = |k: usize| packet.get(i + 4 * k..i + 4 * k + 4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let pos = Position { x: f(0)?, y: f(1)?, z: f(2)? };
    let plausible = |v: f32| v.is_finite() && v.abs() < 5_000_000.0;
    (plausible(pos.x) && plausible(pos.y) && plausible(pos.z) && pos.x.abs().max(pos.y.abs()) > 1_000.0).then_some(pos)
}

fn histogram(name: &str, values: &mut [f32]) {
    values.sort_by(f32::total_cmp);
    let at = |q: f32| values.get(((values.len() as f32 - 1.0) * q) as usize).copied().unwrap_or(f32::NAN);
    eprintln!("{name}: {} â€” p10 {:.0} m, p25 {:.0} m, p50 {:.0} m, p75 {:.0} m, p90 {:.0} m", values.len(), at(0.1), at(0.25), at(0.5), at(0.75), at(0.9));
    let mut buckets = BTreeMap::new();
    for v in values.iter() {
        *buckets.entry(((*v / 10.0) as u32 * 10).min(200)).or_insert(0u32) += 1;
    }
    eprintln!("  {}", buckets.iter().map(|(b, n)| format!("{b}:{n}")).collect::<Vec<_>>().join(" "));
}

#[test]
fn scan_for_your_spot() {
    let Ok(path) = std::env::var("A2_SCAN_CAPTURE") else {
        eprintln!("A2_SCAN_CAPTURE unset; skipping");
        return;
    };
    let storage = Arc::new(DataStorage::new());
    let scan = Arc::new(Mutex::new(Scan::default()));
    {
        let storage = storage.clone();
        let scan = scan.clone();
        frame_tap::set(Some(Arc::new(move |packet: &[u8]| {
            let len = read_varint(packet, 0);
            if len.length <= 0 {
                return;
            }
            let at = len.length as usize;
            let Some(op) = packet.get(at..at + 2) else { return };
            let kind = format!("{:02X} {:02X}", op[0], op[1]);
            let entity = read_varint(packet, at + 2);
            let me_id = storage.local_player_id();
            let now = a2tools_dps_meter_lib::clock::now_ms();
            let fresh = storage.self_position().filter(|(_, age)| *age <= FRESH_MS).map(|(p, _)| p);
            let mut scan = scan.lock();
            *scan.types.entry(kind.clone()).or_default() += 1;

            match positions::parse(packet) {
                Some(Spotted::You(pos)) => {
                    scan.sight.clear();
                    scan.fixes.push((now, pos));
                }
                Some(Spotted::Entity(id, pos)) if Some(id as i64) == me_id => {
                    scan.fixes.push((now, pos));
                    let sight = scan.sight.values().copied().collect();
                    scan.at_fixes.push((now, pos, sight));
                }
                Some(Spotted::Entity(id, pos)) => {
                    scan.sight.insert(id, (pos, now));
                    let new = !scan.seen.contains_key(&id);
                    scan.seen.insert(id, (now, pos));
                    if let (true, Some(me)) = (new, fresh) {
                        scan.appear.entry(kind.clone()).or_default().push(me.metres_to(&pos));
                    }
                }
                None => {}
            }
            if entity.length > 0 && scan.sight.contains_key(&entity.value) {
                scan.last_kind.insert(entity.value, (kind.clone(), now));
            }
            if op == [0x42, 0x36] && entity.length > 0 {
                let flag = packet.get(at + 2 + entity.length as usize + 1).copied().unwrap_or(0xFF);
                if let (Some((pos, _)), Some(me)) = (scan.sight.remove(&entity.value), fresh) {
                    scan.gone.entry(flag).or_default().push(me.metres_to(&pos));
                }
            }
            if op == [0x47, 0x36] && entity.length > 0 {
                scan.sight.remove(&entity.value);
                if let (Some((_, pos)), Some(me)) = (scan.seen.remove(&entity.value), fresh) {
                    scan.leave.push(me.metres_to(&pos));
                }
            }
            if matches!(op, [0x2A, 0x38] | [0x02, 0x38] | [0x23, 0x36] | [0x1A | 0x1B | 0x1C | 0x3C, 0x37] | [0x41, 0x36]) {
                return;
            }
            let is_me = entity.length > 0 && Some(entity.value as i64) == me_id;
            for i in at + 2..packet.len().saturating_sub(11) {
                if let Some(pos) = floats(packet, i) {
                    scan.candidates.push((now, kind.clone(), i - at, is_me, pos));
                }
            }
        })));
    }

    let mut processor = StreamProcessor::new(storage.clone(), Arc::new(SkillLookup::new()), Arc::new(NpcLookup::new()));
    let mut streams: HashMap<String, PacketAccumulator> = HashMap::new();
    let (mut fresh_s, mut stale_s, mut last_second) = (0u32, 0u32, 0);
    let mut centre_error = Vec::new();
    for p in read_capture(std::path::Path::new(&path)).unwrap() {
        a2tools_dps_meter_lib::clock::set_override(Some(p.captured_at_ms));
        processor.set_override_timestamp(Some(p.captured_at_ms));
        let acc = streams.entry(p.stream.clone()).or_insert_with(PacketAccumulator::new);
        acc.append(&p.bytes);
        let used = processor.consume_stream(acc.snapshot());
        if used > 0 {
            acc.discard_bytes(used);
        }
        if p.captured_at_ms / 1000 == last_second {
            continue;
        }
        last_second = p.captured_at_ms / 1000;
        match storage.self_position() {
            Some((me, age)) if age <= FRESH_MS => {
                fresh_s += 1;
                let scan = scan.lock();
                let near: Vec<_> = scan.seen.values().filter(|(at, _)| p.captured_at_ms - at <= SEEN_MS).map(|(_, pos)| *pos).collect();
                if near.len() >= 3 {
                    let n = near.len() as f32;
                    let centre = Position { x: near.iter().map(|p| p.x).sum::<f32>() / n, y: near.iter().map(|p| p.y).sum::<f32>() / n, z: me.z };
                    centre_error.push(me.metres_to(&centre));
                }
            }
            _ => stale_s += 1,
        }
    }
    frame_tap::set(None);
    a2tools_dps_meter_lib::clock::set_override(None);

    let mut scan = scan.lock();
    eprintln!("seconds with your spot fresh: {fresh_s}, stale: {stale_s}; spots told: {}", scan.fixes.len());
    let mut gaps: Vec<f32> = scan.fixes.windows(2).map(|w| (w[1].0 - w[0].0) as f32 / 1000.0).filter(|s| *s * 1000.0 >= GAP_MS as f32).collect();
    eprintln!("gaps of {} s and more: {}, {:.0} s in all", GAP_MS / 1000, gaps.len(), gaps.iter().sum::<f32>());
    histogram("gaps (s, not m)", &mut gaps);

    // Where you could have been at `t`, in a gap: near both ends at running speed.
    let fixes = &scan.fixes;
    let mut near: BTreeMap<(String, usize), (u32, u32)> = BTreeMap::new();
    for (t, kind, offset, is_me, pos) in &scan.candidates {
        let after = fixes.partition_point(|(at, _)| at <= t);
        let (Some((t0, p0)), Some((t1, p1))) = (after.checked_sub(1).and_then(|i| fixes.get(i)), fixes.get(after)) else { continue };
        if t1 - t0 < GAP_MS || t - t0 < 500 || t1 - t < 500 {
            continue;
        }
        let reach = |dt: i64, from: &Position| from.metres_to(pos) * 100.0 <= dt as f32 * SPEED + 500.0 && (from.z - pos.z).abs() < 3_000.0;
        if reach(t - t0, p0) && reach(t1 - t, p1) {
            let e = near.entry((kind.clone(), *offset)).or_default();
            e.0 += 1;
            e.1 += *is_me as u32;
        }
    }
    // A2_SCAN_DUMP="29 37,34 36": every such frame, how far its spots are from
    // where you'd be on a straight line between the spots told around it.
    if let Ok(dump) = std::env::var("A2_SCAN_DUMP") {
        for (t, kind, offset, is_me, pos) in &scan.candidates {
            if !dump.split(',').any(|k| k.trim() == kind) {
                continue;
            }
            let after = fixes.partition_point(|(at, _)| at <= t);
            let (Some((t0, p0)), Some((t1, p1))) = (after.checked_sub(1).and_then(|i| fixes.get(i)), fixes.get(after)) else { continue };
            let f = (*t - t0) as f32 / (t1 - t0).max(1) as f32;
            let line = Position { x: p0.x + (p1.x - p0.x) * f, y: p0.y + (p1.y - p0.y) * f, z: p0.z + (p1.z - p0.z) * f };
            eprintln!(
                "  {kind} @{offset} t+{:>5} ms of {:>5}  me {is_me}  {:.0} m off the line, {:.0} m from the spot before, z {:.0} vs {:.0}",
                t - t0,
                t1 - t0,
                line.metres_to(pos),
                p0.metres_to(pos),
                pos.z,
                line.z
            );
        }
    }
    // How far what's in sight is from you, at each spot of yours told.
    let mut all = Vec::new();
    let mut farthest = Vec::new();
    for (_, me, sight) in &scan.at_fixes {
        let d: Vec<f32> = sight.iter().map(|(p, _)| me.metres_to(p)).collect();
        if let Some(max) = d.iter().copied().reduce(f32::max) {
            farthest.push(max);
        }
        all.extend(d);
    }
    let end = scan.fixes.last().map_or(0, |(t, _)| *t);
    let mut last: BTreeMap<String, u32> = BTreeMap::new();
    for (kind, at) in scan.last_kind.values() {
        if end - at > 60_000 {
            *last.entry(kind.clone()).or_default() += 1;
        }
    }
    let mut last: Vec<_> = last.into_iter().collect();
    last.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    eprintln!("the last frame type naming an entity (not named in the last minute): {last:?}");
    // Each spot of yours told after a gap, guessed without it: from the one
    // before (as now), from the middle of what's in sight, and both.
    let mut errors: BTreeMap<&str, Vec<f32>> = BTreeMap::new();
    for w in scan.at_fixes.windows(2) {
        let ((t0, p0, _), (t1, p1, sight)) = (&w[0], &w[1]);
        let age = t1 - t0;
        if age < GAP_MS || p0.metres_to(p1) > 1_000.0 {
            continue;
        }
        let reach = 5.0 + age as f32 * SPEED / 100.0;
        errors.entry("a: the spot before").or_default().push(p0.metres_to(p1));
        let mean = |ps: &[Position]| -> Option<Position> {
            (!ps.is_empty()).then(|| {
                let n = ps.len() as f32;
                Position { x: ps.iter().map(|p| p.x).sum::<f32>() / n, y: ps.iter().map(|p| p.y).sum::<f32>() / n, z: p0.z }
            })
        };
        // Pulled back into reach of the spot before.
        let clamp = |c: Position| -> Position {
            let d = p0.metres_to(&c);
            if d <= reach {
                return c;
            }
            let f = reach / d;
            Position { x: p0.x + (c.x - p0.x) * f, y: p0.y + (c.y - p0.y) * f, z: p0.z }
        };
        let all: Vec<Position> = sight.iter().map(|(p, _)| *p).collect();
        if all.len() >= 3 {
            let c = mean(&all).unwrap();
            errors.entry("f: (3+ in sight) the spot before").or_default().push(p0.metres_to(p1));
            errors.entry("f: (3+ in sight) middle of all").or_default().push(c.metres_to(p1));
            errors.entry("f: (3+ in sight) middle, within reach").or_default().push(clamp(c).metres_to(p1));
            for full_ms in [5_000.0f32, 10_000.0, 20_000.0] {
                let f = (age as f32 / full_ms).min(1.0);
                let blend = Position { x: p0.x + (c.x - p0.x) * f, y: p0.y + (c.y - p0.y) * f, z: p0.z };
                let name: &'static str = Box::leak(format!("f: (3+ in sight) blend, middle in full at {} s", full_ms / 1000.0).into_boxed_str());
                errors.entry(name).or_default().push(clamp(blend).metres_to(p1));
            }
        }
        let recent: Vec<Position> = sight.iter().filter(|(_, at)| t1 - at <= SEEN_MS).map(|(p, _)| *p).collect();
        let near: Vec<Position> = all.iter().filter(|p| p0.metres_to(p) <= reach + 80.0).copied().collect();
        for (name, set) in [("b: middle of all", &all), ("c: middle of the recent", &recent), ("d: middle of those near the spot before", &near)] {
            if let Some(c) = mean(set) {
                errors.entry(name).or_default().push(c.metres_to(p1));
                errors.entry(match name.as_bytes()[0] {
                    b'b' => "b+: within reach",
                    b'c' => "c+: within reach",
                    _ => "d+: within reach",
                }).or_default().push(clamp(c).metres_to(p1));
            } else {
                for n in ["b: middle of all", "c: middle of the recent", "d: middle of those near the spot before"] {
                    if n == name {
                        errors.entry(match name.as_bytes()[0] {
                            b'b' => "b+: within reach",
                            b'c' => "c+: within reach",
                            _ => "d+: within reach",
                        }).or_default().push(p0.metres_to(p1));
                    }
                }
            }
        }
        // Where most of what's in sight is within R of, within reach (a 5 m grid).
        for r in [60.0f32, 80.0, 100.0] {
            let step = 500.0;
            let cells = (reach * 100.0 / step).ceil() as i32;
            let mut best = (0usize, Vec::new());
            for i in -cells..=cells {
                for j in -cells..=cells {
                    let c = Position { x: p0.x + i as f32 * step, y: p0.y + j as f32 * step, z: p0.z };
                    if p0.metres_to(&c) > reach {
                        continue;
                    }
                    let n = all.iter().filter(|p| c.metres_to(p) <= r).count();
                    if n > best.0 {
                        best = (n, vec![c]);
                    } else if n == best.0 {
                        best.1.push(c);
                    }
                }
            }
            let guess = mean(&best.1).unwrap_or(*p0);
            let name: &'static str = Box::leak(format!("e: most in sight within {r} m, within reach").into_boxed_str());
            errors.entry(name).or_default().push(guess.metres_to(p1));
        }
    }
    for (name, values) in errors.iter_mut() {
        histogram(name, values);
    }
    for (flag, values) in scan.gone.iter_mut() {
        histogram(&format!("42 36 flag {flag}"), values);
    }
    histogram("in sight, from you", &mut all);
    histogram("farthest in sight, from you", &mut farthest);
    eprintln!("frame types with a spot where you could have been, in gaps (type @offset: frames / of which the entity was you / frames of that type):");
    let mut near: Vec<_> = near.into_iter().collect();
    near.sort_by_key(|(_, (n, _))| std::cmp::Reverse(*n));
    for ((kind, offset), (n, me)) in near.into_iter().take(30) {
        eprintln!("  {kind} @{offset:<3} {n:>5} / {me:>5} / {}", scan.types.get(&kind).copied().unwrap_or(0));
    }
    for (kind, values) in scan.appear.iter_mut() {
        histogram(&format!("into sight by {kind}"), values);
    }
    let mut leave = std::mem::take(&mut scan.leave);
    histogram("out of sight", &mut leave);
    histogram("middle of what's in sight, from you", &mut centre_error);
}
