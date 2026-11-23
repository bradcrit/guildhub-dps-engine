//! Where everyone was over a fight, for a replay (GuildHub Companion): the
//! spots the game told (`capture::positions`) of you, the fight's players,
//! its boss and the mobs and helper NPCs around it, and the NPC skill casts
//! aimed at a spot (a boss's ground mechanics).
//!
//! The players' HP over the fight too (`1b 92` for the party, `00 8d` for
//! you and players named): each change, kept as a share of their maximum.
//!
//! Not every record tells a spot as well: a move (`1A/1B/1C/3C 37`), a spawn
//! or a teleport is where the entity is, but an effect's spot (`2A 38`) or a
//! skill's target spot (`02 38`) is off by 5 m and more one time in ten
//! (`tests/track_jitter_scan.rs`, 2026-11-18: p90 5.4 m off the moves' path,
//! back and forth) — a replay's players jumped. A saved track keeps those only
//! where no move was told near them, and drops a spot that goes off and comes
//! straight back faster than anyone runs (`clean`).
//!
//! Every spot is logged as it comes (at most one per entity every `STEP_MS`
//! unless it jumped), for the last `KEEP_MS`; a saved fight takes its window
//! from the log (`DataStorage::fight_track`).
//!
//! Measured on two captures (2026-11-18, 2026-11-22, `tests/replay_density_scan.rs`):
//! your spot comes every 100–250 ms (median), a party member's every
//! 100–200 ms, a mob's ~100 ms while it is hit, with gaps of a second at most
//! in a fight — enough to draw everyone moving.

use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};

use crate::capture::positions::Position;

/// The log keeps this long.
pub const KEEP_MS: i64 = 20 * 60_000;
/// And at most this many spots (a crowd in the open world).
const MAX_POINTS: usize = 400_000;
const MAX_CASTS: usize = 40_000;
const MAX_HP_POINTS: usize = 200_000;
/// HP samples per player in a saved fight; more keep every n-th.
const MAX_ENTITY_HP: usize = 6_000;
/// One spot per entity this often, unless it moved further than `JUMP_CM`.
const STEP_MS: i64 = 150;
const JUMP_CM: f32 = 300.0;
/// A spot not from a move is dropped within this of one that is.
const TRUST_MS: i64 = 700;
/// A spike: off by more than this plus running speed for the time between.
const SPIKE_CM: f32 = 250.0;
const RUN_CM_PER_MS: f32 = 0.8;
/// Your spot, when the game tells it without your id (`Spotted::You`) or
/// before your id is known.
pub const SELF_KEY: i32 = 0;
/// Entities kept around the fight: within this of the boss's (else your) spots.
const AROUND_CM: f32 = 9_000.0;
/// At most this many mobs and NPCs in a saved fight, the most spots first.
const MAX_OTHERS: usize = 80;
/// Spots per entity in a saved fight; a longer track keeps every n-th.
const MAX_ENTITY_POINTS: usize = 8_000;
const MAX_SAVED_CASTS: usize = 6_000;
/// A fight's track starts this long before the first hit and ends this long after the last.
pub const LEAD_MS: i64 = 5_000;
pub const TAIL_MS: i64 = 3_000;

/// What an entity was when its spot was logged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Player,
    Mob,
    Other,
}

#[derive(Debug, Clone, Copy)]
struct Point {
    at: i64,
    id: i32,
    pos: Position,
    /// From a move, spawn or teleport record: where the entity is.
    exact: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct Cast {
    pub at: i64,
    pub caster: i32,
    pub skill: u32,
    pub target: i32,
    pub pos: Position,
}

#[derive(Default)]
pub struct TrackLog {
    points: VecDeque<Point>,
    last: HashMap<i32, (i64, Position, bool)>,
    /// Entity → (its kind, its NPC code — 0 when none), as last logged.
    kinds: HashMap<i32, (Kind, i32)>,
    casts: VecDeque<Cast>,
    /// (time, entity, HP, max HP — 0 when not known then).
    hp: VecDeque<(i64, i32, i32, i32)>,
    last_hp: HashMap<i32, i32>,
}

impl TrackLog {
    /// An entity's HP as the game told it; only changes are kept.
    pub fn note_hp(&mut self, at: i64, id: i32, hp: i32, max: i32) {
        if self.last_hp.insert(id, hp) == Some(hp) {
            return;
        }
        self.hp.push_back((at, id, hp, max));
        while self.hp.front().is_some_and(|h| at - h.0 > KEEP_MS) || self.hp.len() > MAX_HP_POINTS {
            self.hp.pop_front();
        }
    }

    /// `exact`: told by a move, spawn or teleport (see the module's doc).
    pub fn note(&mut self, at: i64, id: i32, pos: Position, kind: Kind, code: i32, exact: bool) {
        if let Some((t, p, was_exact)) = self.last.get(&id) {
            let moved = ((p.x - pos.x).powi(2) + (p.y - pos.y).powi(2)).sqrt();
            if at - t < STEP_MS && moved < JUMP_CM && (*was_exact || !exact) {
                return;
            }
        }
        self.last.insert(id, (at, pos, exact));
        if kind != Kind::Other || code != 0 || !self.kinds.contains_key(&id) {
            self.kinds.insert(id, (kind, code));
        }
        self.points.push_back(Point { at, id, pos, exact });
        while self.points.front().is_some_and(|p| at - p.at > KEEP_MS) || self.points.len() > MAX_POINTS {
            self.points.pop_front();
        }
    }

    pub fn note_cast(&mut self, cast: Cast) {
        let at = cast.at;
        self.casts.push_back(cast);
        while self.casts.front().is_some_and(|c| at - c.at > KEEP_MS) || self.casts.len() > MAX_CASTS {
            self.casts.pop_front();
        }
    }

    /// A map load: the entity ids start over, so what was logged under one
    /// no longer says what it is.
    pub fn map_loaded(&mut self) {
        self.last.clear();
        self.kinds.clear();
        self.last_hp.clear();
    }

    fn kind(&self, id: i32) -> (Kind, i32) {
        self.kinds.get(&id).copied().unwrap_or((Kind::Other, 0))
    }
}

/// A fight's track as saved with it (`FightRecord::track`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FightTrack {
    /// The format: 1.
    pub v: u8,
    pub ents: Vec<TrackEntity>,
    /// NPC casts at a spot, `t,caster,skill,x,y,z;…` — `t` in ms from the
    /// fight's start, the spot in decimetres (the game's units / 10).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub casts: String,
    /// Each player's HP over the fight, by actor id: `t,permille;…` — the
    /// share of their max HP in thousandths, `t` as in `pts`. The HP before
    /// the window, when known, comes first at the window's start.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub hp: std::collections::BTreeMap<i32, String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrackEntity {
    /// An actor's id (as in `FightRecord::actors`), the boss's, or a mob's.
    pub id: i32,
    /// "you", "player", "boss", "mob" or "npc".
    pub kind: String,
    /// The NPC code (mobs, bosses, NPCs); 0 for players.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub code: i32,
    /// `t,x,y,z;…` — `t` in ms from the fight's start (negative before the
    /// first hit), the spot in decimetres.
    pub pts: String,
}

fn is_zero(n: &i32) -> bool {
    *n == 0
}

fn dm(v: f32) -> i32 {
    (v / 10.0).round() as i32
}

fn encode(points: &[(i64, Position)], start: i64) -> String {
    let step = points.len().div_ceil(MAX_ENTITY_POINTS).max(1);
    let mut out = String::with_capacity(points.len() / step * 24);
    for (i, (t, p)) in points.iter().enumerate() {
        if i % step != 0 && i + 1 != points.len() {
            continue;
        }
        if !out.is_empty() {
            out.push(';');
        }
        out.push_str(&format!("{},{},{},{}", t - start, dm(p.x), dm(p.y), dm(p.z)));
    }
    out
}

fn dist(a: &Position, b: &Position) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

/// One entity's spots, oldest first, without the ones that make it jump: a
/// spot not from a move near one that is, and spikes — one or two spots off
/// and straight back, further than running gets in the time.
fn clean(mut v: Vec<(i64, Position, bool)>) -> Vec<(i64, Position)> {
    v.sort_by_key(|p| p.0);
    let exact: Vec<i64> = v.iter().filter(|p| p.2).map(|p| p.0).collect();
    if !exact.is_empty() {
        v.retain(|p| {
            p.2 || {
                let i = exact.partition_point(|&t| t < p.0 - TRUST_MS);
                exact.get(i).is_none_or(|&t| t > p.0 + TRUST_MS)
            }
        });
    }
    let mut pts: Vec<(i64, Position)> = v.into_iter().map(|p| (p.0, p.1)).collect();
    let reach = |dt: i64| SPIKE_CM + dt.max(0) as f32 * RUN_CM_PER_MS;
    for _ in 0..4 {
        let mut keep = vec![true; pts.len()];
        let mut any = false;
        let mut i = 1;
        while i + 1 < pts.len() {
            let a = pts[i - 1];
            let mut dropped = false;
            for span in 1..=2usize {
                let Some(&c) = pts.get(i + span) else { break };
                if dist(&a.1, &c.1) > reach(c.0 - a.0) {
                    continue;
                }
                let off = (i..i + span).all(|j| dist(&pts[j].1, &a.1) > reach(pts[j].0 - a.0) && dist(&pts[j].1, &c.1) > reach(c.0 - pts[j].0));
                if off {
                    for k in keep.iter_mut().skip(i).take(span) {
                        *k = false;
                    }
                    i += span;
                    dropped = true;
                    any = true;
                    break;
                }
            }
            if !dropped {
                i += 1;
            }
        }
        if !any {
            break;
        }
        let mut k = keep.into_iter();
        pts.retain(|_| k.next().unwrap_or(true));
    }
    pts
}

fn centre(points: &[(i64, Position)]) -> Option<Position> {
    if points.is_empty() {
        return None;
    }
    let n = points.len() as f32;
    Some(Position {
        x: points.iter().map(|(_, p)| p.x).sum::<f32>() / n,
        y: points.iter().map(|(_, p)| p.y).sum::<f32>() / n,
        z: points.iter().map(|(_, p)| p.z).sum::<f32>() / n,
    })
}

fn near(a: &Position, b: &Position) -> bool {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt() <= AROUND_CM
}

/// The track of a fight from `start` to `end` (first and last hit, unix ms).
///
/// `me`: your id then; `actors`: the fight's players; `bosses`: the target
/// and any other boss. Players who weren't in the fight are left out; mobs
/// and NPCs only near the boss.
#[allow(clippy::too_many_arguments)]
pub fn for_fight(
    log: &TrackLog,
    start: i64,
    end: i64,
    me: Option<i32>,
    actors: &[i32],
    bosses: &[i32],
    code_of: impl Fn(i32) -> i32,
    max_hp_of: impl Fn(i32) -> i32,
) -> Option<FightTrack> {
    let (from, to) = (start - LEAD_MS, end + TAIL_MS);
    let mut raw: HashMap<i32, Vec<(i64, Position, bool)>> = HashMap::new();
    for p in log.points.iter().filter(|p| p.at >= from && p.at <= to) {
        let id = if p.id == SELF_KEY { me.unwrap_or(SELF_KEY) } else { p.id };
        raw.entry(id).or_default().push((p.at, p.pos, p.exact));
    }
    let by_id: HashMap<i32, Vec<(i64, Position)>> = raw.into_iter().map(|(id, v)| (id, clean(v))).collect();
    let casters: std::collections::HashSet<i32> = log.casts.iter().filter(|c| c.at >= from && c.at <= to).map(|c| c.caster).collect();
    let anchor = bosses
        .iter()
        .find_map(|b| by_id.get(b).and_then(|v| centre(v)))
        .or_else(|| me.and_then(|m| by_id.get(&m)).and_then(|v| centre(v)))?;

    let mut ents = Vec::new();
    let mut others: Vec<(usize, TrackEntity)> = Vec::new();
    for (id, points) in &by_id {
        let (kind, code) = log.kind(*id);
        let label = if Some(*id) == me {
            "you"
        } else if bosses.contains(id) {
            "boss"
        } else if actors.contains(id) {
            "player"
        } else if kind == Kind::Player {
            continue;
        } else if kind == Kind::Mob {
            "mob"
        } else {
            "npc"
        };
        // Only what's near the boss: a party teleported in at the pull was
        // a kilometre away a few seconds before (Transcendent Bakarma, 2026-11-18).
        let points: Vec<(i64, Position)> = points.iter().copied().filter(|(_, p)| near(p, &anchor)).collect();
        if points.is_empty() {
            continue;
        }
        // Something told of a few times with no code and no cast (an object,
        // a stranger's summon) says nothing in a replay.
        if label == "npc" && code == 0 && !casters.contains(id) {
            continue;
        }
        let entity = TrackEntity {
            id: *id,
            kind: label.to_string(),
            code: match (label, code) {
                ("you" | "player", _) => 0,
                (_, 0) => code_of(*id),
                _ => code,
            },
            pts: encode(&points, start),
        };
        if matches!(label, "mob" | "npc") {
            others.push((points.len(), entity));
        } else {
            ents.push(entity);
        }
    }
    // Helper NPCs that stand still and only cast: their spot is the cast's.
    let casts: Vec<&Cast> = log
        .casts
        .iter()
        .filter(|c| c.at >= from && c.at <= to)
        .filter(|c| !actors.contains(&c.caster) && Some(c.caster) != me && log.kind(c.caster).0 != Kind::Player)
        .filter(|c| near(&c.pos, &anchor))
        .take(MAX_SAVED_CASTS)
        .collect();
    others.sort_by(|a, b| b.0.cmp(&a.0));
    ents.extend(others.into_iter().take(MAX_OTHERS).map(|(_, e)| e));
    ents.sort_by_key(|e| (["you", "player", "boss", "mob", "npc"].iter().position(|k| *k == e.kind), e.id));

    let mut cast_text = String::new();
    for c in casts {
        if !cast_text.is_empty() {
            cast_text.push(';');
        }
        cast_text.push_str(&format!("{},{},{},{},{},{}", c.at - start, c.caster, c.skill, dm(c.pos.x), dm(c.pos.y), dm(c.pos.z)));
    }
    let hp = hp_tracks(log, from, to, start, me, actors, max_hp_of);
    (!ents.is_empty()).then_some(FightTrack { v: 1, ents, casts: cast_text, hp })
}

/// The players' HP in the window, as thousandths of their max HP.
fn hp_tracks(
    log: &TrackLog,
    from: i64,
    to: i64,
    start: i64,
    me: Option<i32>,
    actors: &[i32],
    max_hp_of: impl Fn(i32) -> i32,
) -> std::collections::BTreeMap<i32, String> {
    let wanted = |id: i32| actors.contains(&id) || Some(id) == me;
    let mut by_id: HashMap<i32, Vec<(i64, i32, i32)>> = HashMap::new();
    let mut before: HashMap<i32, (i32, i32)> = HashMap::new();
    for &(at, id, hp, max) in &log.hp {
        if !wanted(id) || at > to {
            continue;
        }
        if at < from {
            before.insert(id, (hp, max));
        } else {
            by_id.entry(id).or_default().push((at, hp, max));
        }
    }
    let mut out = std::collections::BTreeMap::new();
    for id in actors.iter().copied().chain(me) {
        if out.contains_key(&id) {
            continue;
        }
        let mut rows: Vec<(i64, i32, i32)> = before.get(&id).map(|&(hp, max)| vec![(from, hp, max)]).unwrap_or_default();
        rows.extend(by_id.remove(&id).unwrap_or_default());
        if rows.is_empty() {
            continue;
        }
        // A max told along (`1b 92`) holds until the next; else the
        // highest known (yours from the login record).
        let fallback = rows.iter().map(|r| r.2).max().unwrap_or(0).max(max_hp_of(id)).max(rows.iter().map(|r| r.1).max().unwrap_or(0));
        if fallback <= 0 {
            continue;
        }
        let step = rows.len().div_ceil(MAX_ENTITY_HP).max(1);
        let mut text = String::new();
        let mut max = 0;
        for (i, &(at, hp, m)) in rows.iter().enumerate() {
            if m > 0 {
                max = m;
            }
            if i % step != 0 && i + 1 != rows.len() {
                continue;
            }
            let base = if max > 0 { max.max(hp) } else { fallback };
            let permille = ((hp.max(0) as i64 * 1000) / base as i64).clamp(0, 1000);
            if !text.is_empty() {
                text.push(';');
            }
            text.push_str(&format!("{},{}", at - start, permille));
        }
        out.insert(id, text);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(x: f32, y: f32) -> Position {
        Position { x, y, z: 100.0 }
    }

    #[test]
    fn keeps_the_fight_and_whats_near_it() {
        let mut log = TrackLog::default();
        for i in 0..20 {
            let t = 10_000 + i * 200;
            log.note(t, SELF_KEY, at(1_000.0 + i as f32 * 10.0, 2_000.0), Kind::Player, 0, true);
            log.note(t, 7, at(1_500.0, 2_000.0), Kind::Player, 0, true);
            log.note(t, 50, at(1_600.0, 2_100.0), Kind::Mob, 2300371, true);
            log.note(t, 60, at(1_700.0, 2_100.0), Kind::Mob, 123, true);
            log.note(t, 61, at(900_000.0, 2_100.0), Kind::Mob, 124, true);
            log.note(t, 99, at(1_400.0, 2_000.0), Kind::Player, 0, true);
        }
        log.note_cast(Cast { at: 11_000, caster: 70, skill: 1801182, target: 70, pos: at(1_650.0, 2_050.0) });
        log.note_hp(4_000, 5, 900, 1_000);
        log.note_hp(11_000, 5, 450, 0);
        log.note_hp(11_500, 5, 450, 0);
        log.note_hp(12_000, 7, 0, 2_000);
        let track = for_fight(&log, 10_500, 13_000, Some(5), &[5, 7], &[50], |_| 0, |_| 0).unwrap();
        assert_eq!(track.hp.get(&5).map(String::as_str), Some("-5000,900;500,450"));
        assert_eq!(track.hp.get(&7).map(String::as_str), Some("1500,0"));
        let kinds: Vec<(&str, i32)> = track.ents.iter().map(|e| (e.kind.as_str(), e.id)).collect();
        assert_eq!(kinds, vec![("you", 5), ("player", 7), ("boss", 50), ("mob", 60)]);
        assert_eq!(track.ents[2].code, 2300371);
        assert!(track.ents[0].pts.starts_with("-500,100,200,10;"));
        assert_eq!(track.casts, "500,70,1801182,165,205,10");
    }

    #[test]
    fn effect_spots_near_moves_and_spikes_are_dropped() {
        let p = |x: f32| at(x, 0.0);
        // Moves every 300 ms walking +1 m; an effect spot 10 m off between two moves.
        let v = vec![(0, p(0.0), true), (300, p(100.0), true), (450, p(1_000.0), false), (600, p(200.0), true)];
        assert_eq!(clean(v).iter().map(|q| q.0).collect::<Vec<_>>(), vec![0, 300, 600]);
        // Effects only (you): a spike of two spots 8 m off and back is dropped; a real 20 m dash that stays isn't.
        let v = vec![
            (0, p(0.0), false), (200, p(50.0), false), (400, p(900.0), false), (500, p(905.0), false),
            (700, p(120.0), false), (900, p(150.0), false), (1_000, p(2_150.0), false), (1_300, p(2_200.0), false),
        ];
        assert_eq!(clean(v).iter().map(|q| q.0).collect::<Vec<_>>(), vec![0, 200, 700, 900, 1_000, 1_300]);
    }

    #[test]
    fn throttles_but_keeps_jumps() {
        let mut log = TrackLog::default();
        log.note(0, 1, at(0.0, 0.0), Kind::Mob, 0, true);
        log.note(50, 1, at(10.0, 0.0), Kind::Mob, 0, true);
        log.note(100, 1, at(5_000.0, 0.0), Kind::Mob, 0, true);
        log.note(400, 1, at(5_010.0, 0.0), Kind::Mob, 0, true);
        assert_eq!(log.points.len(), 3);
    }
}
