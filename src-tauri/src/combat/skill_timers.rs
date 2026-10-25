//! What the skill records (`capture::skill_casts`) add up to: your skills'
//! cooldowns, what each monster is casting and when it lands, and the time
//! limit of a boss that has one — for GuildHub Companion's cooldown bar and
//! cast bar.
//!
//! A monster's cast time isn't in its record, so it is learned: the time from
//! a monster's `02 38` to its first hit with that skill (`note_hit`), kept per
//! skill as the median of the last few — the same skill took the same time to
//! within ±60 ms in our captures. The learned times outlive the session
//! (`learned`, `set_learned`), so a boss's second pull already has them.

use std::collections::{HashMap, VecDeque};

use crate::capture::skill_casts::{base_skill, is_npc_skill, Record};

/// A hit later than this after a cast isn't that cast's.
const MAX_CAST_MS: i64 = 6_000;
/// Samples kept per skill.
const SAMPLES: usize = 5;
/// A cast with no learned time is shown this long at most.
const UNKNOWN_CAST_SHOWN_MS: i64 = 3_000;
/// Most skills whose cast times are kept.
const MAX_LEARNED: usize = 5_000;
/// Casts held per monster, oldest dropped: an open-world fight has dozens.
const MAX_CASTERS: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cooldown {
    /// The skill without its specialization digits (`base_skill`).
    pub skill: u32,
    pub ends_ms: i64,
    pub total_ms: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NpcCast {
    pub skill: u32,
    pub target: i32,
    pub started_ms: i64,
    /// The learned cast time, None while not known.
    pub cast_ms: Option<u32>,
}

#[derive(Default)]
pub struct SkillTimers {
    /// Your cooldowns, by base skill.
    cooldowns: HashMap<u32, Cooldown>,
    /// Each monster's latest cast; `landed` once its first hit came.
    casts: HashMap<i32, (NpcCast, bool)>,
    /// Cast time samples (ms) per monster skill, newest last.
    samples: HashMap<u32, VecDeque<u32>>,
    /// Bumped whenever `samples` changes, for saving (`learned_since`).
    seq: u64,
    /// Time limits of the bosses pulled (unix ms).
    deadlines: HashMap<i32, i64>,
}

impl SkillTimers {
    /// One skill record at `now_ms`; `me` is your entity, when known.
    pub fn note(&mut self, record: Record, now_ms: i64, me: Option<i32>) {
        match record {
            Record::Cast(cast) => {
                if Some(cast.actor) == me {
                    if cast.cooldown_ms > 0 {
                        let skill = base_skill(cast.skill);
                        self.cooldowns.insert(skill, Cooldown { skill, ends_ms: now_ms + i64::from(cast.cooldown_ms), total_ms: cast.cooldown_ms });
                    }
                } else if is_npc_skill(cast.skill) {
                    if self.casts.len() >= MAX_CASTERS && !self.casts.contains_key(&cast.actor) {
                        let oldest = self.casts.iter().min_by_key(|(_, (c, _))| c.started_ms).map(|(&id, _)| id);
                        if let Some(id) = oldest {
                            self.casts.remove(&id);
                        }
                    }
                    let cast_ms = self.cast_time(cast.skill);
                    self.casts.insert(cast.actor, (NpcCast { skill: cast.skill, target: cast.target, started_ms: now_ms, cast_ms }, false));
                }
            }
            Record::Cooldowns(list) => {
                for (skill, left) in list {
                    let skill = base_skill(skill);
                    if left == 0 {
                        self.cooldowns.remove(&skill);
                        continue;
                    }
                    // A reduction keeps the bar's length; a cooldown first
                    // heard of here (at login) is as long as what's left.
                    let total_ms = self.cooldowns.get(&skill).map_or(left, |c| c.total_ms.max(left));
                    self.cooldowns.insert(skill, Cooldown { skill, ends_ms: now_ms + i64::from(left), total_ms });
                }
            }
            Record::Battle { entity, on, deadline_ms } => match (on, deadline_ms) {
                (true, Some(deadline)) => {
                    self.deadlines.insert(entity, deadline);
                }
                (false, _) => {
                    self.deadlines.remove(&entity);
                }
                _ => {}
            },
        }
    }

    /// A hit by `actor` with `skill` at `now_ms`: the first after its cast
    /// says how long the cast took.
    pub fn note_hit(&mut self, actor: i32, skill: u32, now_ms: i64) {
        let Some((cast, landed)) = self.casts.get_mut(&actor) else { return };
        if *landed || cast.skill != skill {
            return;
        }
        *landed = true;
        let took = now_ms - cast.started_ms;
        if !(0..=MAX_CAST_MS).contains(&took) {
            return;
        }
        if self.samples.len() >= MAX_LEARNED && !self.samples.contains_key(&skill) {
            return;
        }
        let samples = self.samples.entry(skill).or_default();
        samples.push_back(took as u32);
        if samples.len() > SAMPLES {
            samples.pop_front();
        }
        self.seq += 1;
    }

    /// The learned cast time of a monster skill: the median of its samples.
    pub fn cast_time(&self, skill: u32) -> Option<u32> {
        let samples = self.samples.get(&skill)?;
        let mut sorted: Vec<u32> = samples.iter().copied().collect();
        sorted.sort_unstable();
        sorted.get(sorted.len() / 2).copied()
    }

    /// Your cooldowns still running at `now_ms`, the soonest back first.
    pub fn cooldowns(&mut self, now_ms: i64) -> Vec<Cooldown> {
        self.cooldowns.retain(|_, c| c.ends_ms > now_ms);
        let mut out: Vec<Cooldown> = self.cooldowns.values().copied().collect();
        out.sort_by_key(|c| (c.ends_ms, c.skill));
        out
    }

    /// What `entity` is casting at `now_ms`: its latest cast while it hasn't
    /// landed — up to its learned time (plus a little), or
    /// UNKNOWN_CAST_SHOWN_MS while the time isn't known.
    pub fn casting(&self, entity: i32, now_ms: i64) -> Option<NpcCast> {
        let &(cast, landed) = self.casts.get(&entity)?;
        let shown = cast.cast_ms.map_or(UNKNOWN_CAST_SHOWN_MS, |ms| i64::from(ms) + 250);
        (!landed && now_ms - cast.started_ms <= shown).then_some(cast)
    }

    /// The time limit of `entity`, a boss pulled that has one (unix ms).
    pub fn deadline(&self, entity: i32) -> Option<i64> {
        self.deadlines.get(&entity).copied()
    }

    /// Every learned cast time (skill → ms) when they changed since `seq`, with
    /// the new seq.
    pub fn learned_since(&self, seq: u64) -> Option<(u64, HashMap<u32, Vec<u32>>)> {
        (self.seq != seq).then(|| (self.seq, self.samples.iter().map(|(&k, v)| (k, v.iter().copied().collect())).collect()))
    }

    /// Cast times saved by an earlier session (`learned_since`).
    pub fn set_learned(&mut self, learned: HashMap<u32, Vec<u32>>) {
        self.samples = learned
            .into_iter()
            .filter(|(skill, _)| is_npc_skill(*skill))
            .take(MAX_LEARNED)
            .map(|(skill, v)| {
                let mut kept: VecDeque<u32> = v.into_iter().filter(|&ms| i64::from(ms) <= MAX_CAST_MS).collect();
                while kept.len() > SAMPLES {
                    kept.pop_front();
                }
                (skill, kept)
            })
            .filter(|(_, kept)| !kept.is_empty())
            .collect();
    }

    /// A map load: the monsters cast nothing anymore, their limits are gone.
    pub fn map_load(&mut self) {
        self.casts.clear();
        self.deadlines.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::skill_casts::Cast;

    fn cast(actor: i32, skill: u32, cooldown_ms: u32) -> Record {
        Record::Cast(Cast { actor, skill, target: 7, cooldown_ms, speed: 10_000 })
    }

    #[test]
    fn your_casts_go_on_cooldown_and_the_server_moves_them() {
        let mut t = SkillTimers::default();
        t.note(cast(7, 18_010_140, 8_000), 1_000, Some(7));
        t.note(cast(8, 18_020_140, 8_000), 1_000, Some(7));
        assert_eq!(t.cooldowns(1_000), [Cooldown { skill: 18_010_000, ends_ms: 9_000, total_ms: 8_000 }]);
        t.note(Record::Cooldowns(vec![(18_010_140, 2_000)]), 3_000, Some(7));
        assert_eq!(t.cooldowns(3_000), [Cooldown { skill: 18_010_000, ends_ms: 5_000, total_ms: 8_000 }]);
        t.note(Record::Cooldowns(vec![(18_010_000, 0)]), 3_500, Some(7));
        assert!(t.cooldowns(3_500).is_empty());
        t.note(cast(7, 10_000, 0), 4_000, Some(7));
        assert!(t.cooldowns(9_000).is_empty());
    }

    #[test]
    fn a_monster_cast_time_is_learned_from_its_first_hit() {
        let mut t = SkillTimers::default();
        t.note(cast(500, 1_228_880, 1_000), 10_000, Some(7));
        assert_eq!(t.casting(500, 10_100).map(|c| c.cast_ms), Some(None));
        t.note_hit(500, 1_228_880, 10_606);
        t.note_hit(500, 1_228_880, 10_900);
        assert_eq!(t.casting(500, 10_700), None, "landed");
        for (at, took) in [(20_000, 642), (30_000, 611)] {
            t.note(cast(500, 1_228_880, 1_000), at, Some(7));
            t.note_hit(500, 1_228_880, at + took);
        }
        assert_eq!(t.cast_time(1_228_880), Some(611));
        t.note(cast(501, 1_228_880, 1_000), 40_000, Some(7));
        assert_eq!(t.casting(501, 40_300), Some(NpcCast { skill: 1_228_880, target: 7, started_ms: 40_000, cast_ms: Some(611) }));
        assert_eq!(t.casting(501, 41_000), None, "past its time");
        // A player's skill is no cast to show.
        t.note(cast(502, 16_110_000, 14_500), 40_000, Some(7));
        assert_eq!(t.casting(502, 40_100), None);
    }

    #[test]
    fn learned_times_survive_a_restart() {
        let mut t = SkillTimers::default();
        t.note(cast(500, 1_233_610, 0), 0, None);
        t.note_hit(500, 1_233_610, 1_154);
        let (seq, learned) = t.learned_since(0).unwrap();
        assert!(t.learned_since(seq).is_none());
        let mut next = SkillTimers::default();
        next.set_learned(learned);
        assert_eq!(next.cast_time(1_233_610), Some(1_154));
    }

    #[test]
    fn a_timed_boss_keeps_its_limit_until_out_of_combat() {
        let mut t = SkillTimers::default();
        t.note(Record::Battle { entity: 900, on: true, deadline_ms: Some(1_000_000) }, 0, None);
        t.note(Record::Battle { entity: 900, on: true, deadline_ms: None }, 1, None);
        assert_eq!(t.deadline(900), Some(1_000_000));
        t.note(Record::Battle { entity: 900, on: false, deadline_ms: None }, 2, None);
        assert_eq!(t.deadline(900), None);
    }
}
