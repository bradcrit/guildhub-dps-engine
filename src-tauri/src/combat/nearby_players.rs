//! The players around you, for the PvP view: who of the other faction is in
//! sight, and who of them is attacking you.
//!
//! Everything here comes from the server's records about other players: their
//! spawn (`45 36`, with class, faction and item level), their movement and
//! skills (last seen) and their despawn (`47 36`). Where you are is not known
//! (your own movement is only in what the client sends, which is encrypted),
//! so there is no distance: the view orders by attack and by last seen.
//!
//! The damage between you and them comes from the hits (`DataStorage::
//! append_damage`): what you dealt each, what each dealt you. Their hits on
//! you of the last `RECAP_WINDOW_MS` are kept, so that when you die the death
//! recap can say who killed you, and with what.
//!
//! The game sends no HP 0 when you die to players: your last HP was 625, then
//! hits came and nothing more until your revive 24 s later (Bradcrit in the
//! Abyss, 17:05:32 on 2026-10-18). So a death is your last HP minus the hits
//! after it reaching 0, once no HP came within `DEATH_CONFIRM_MS` to say a
//! shield took them.

use std::collections::{HashMap, VecDeque};

use crate::entity::job_class::JobClass;

/// The Abyss: map 20 in the map loads (`21 36`) of both trips there on
/// 2026-10-18 (01:29 and 17:04), the home map 1110 before and after.
pub const ABYSS_MAPS: [i32; 1] = [20];

/// Nothing heard of a player for this long: gone, though no despawn came (it
/// can be missed, mid-capture or in a lost packet).
pub const FORGET_AFTER_MS: i64 = 10 * 60_000;

/// A death recap counts the players' hits on you of this long before you died.
pub const RECAP_WINDOW_MS: i64 = 15_000;

/// The hits since your last HP came to more than it: dead, unless a new HP
/// comes within this long (a shield took them). Your HP came every 100-400 ms
/// while you were hit.
pub const DEATH_CONFIRM_MS: i64 = 1_500;

#[derive(Debug, Clone, PartialEq)]
pub struct NearbyPlayer {
    pub id: i32,
    pub name: String,
    pub class: Option<JobClass>,
    /// Equipment item level, as the inspect window shows it.
    pub item_level: Option<u32>,
    /// The spawn's faction byte; yours is in your self record.
    pub faction: u8,
    pub last_seen_ms: i64,
    /// The last time they hit you.
    pub attacked_me_ms: Option<i64>,
    /// Your damage on them since they came into sight.
    pub dealt: u64,
    /// Their damage on you since they came into sight.
    pub taken: u64,
}

/// One hit of a player on you, for the death recap.
#[derive(Debug, Clone, PartialEq)]
struct HitOnMe {
    at_ms: i64,
    actor: i32,
    skill: i32,
    damage: u64,
}

/// What killed you: the players who hit you in the `RECAP_WINDOW_MS` before.
#[derive(Debug, Clone, PartialEq)]
pub struct DeathRecap {
    pub at_ms: i64,
    /// Who landed the last hit, and with what skill.
    pub killer: String,
    pub killing_skill: i32,
    /// Most damage first.
    pub attackers: Vec<RecapAttacker>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecapAttacker {
    pub name: String,
    pub class: Option<JobClass>,
    pub damage: u64,
    pub hits: u32,
    /// The skill that did the most of their damage.
    pub top_skill: i32,
}

#[derive(Default)]
pub struct NearbyPlayers {
    players: HashMap<i32, NearbyPlayer>,
    self_faction: Option<u8>,
    hits_on_me: VecDeque<HitOnMe>,
    last_death: Option<DeathRecap>,
    /// Your last HP less the hits since — None until the game sent one.
    my_hp: Option<i64>,
    /// When the hits took `my_hp` to 0, until confirmed or a new HP came.
    pending_death_ms: Option<i64>,
    /// Down, until an HP above 0 says you are back up.
    dead: bool,
}

impl NearbyPlayers {
    /// Your faction, from your self record.
    pub fn set_self_faction(&mut self, faction: u8) {
        self.self_faction = Some(faction);
    }

    /// A player spawned in sight. The same entity again (a second copy of
    /// the record) keeps when it last attacked you and the damage between you.
    pub fn spawn(&mut self, id: i32, name: &str, class: Option<JobClass>, item_level: Option<u32>, faction: u8, now_ms: i64) {
        let same = self.players.get(&id).filter(|p| p.name == name);
        let attacked_me_ms = same.and_then(|p| p.attacked_me_ms);
        let item_level = item_level.or_else(|| same.and_then(|p| p.item_level));
        let (dealt, taken) = same.map_or((0, 0), |p| (p.dealt, p.taken));
        self.players.insert(
            id,
            NearbyPlayer { id, name: name.to_string(), class, item_level, faction, last_seen_ms: now_ms, attacked_me_ms, dealt, taken },
        );
    }

    /// A player moved or used a skill.
    pub fn seen(&mut self, id: i32, now_ms: i64) {
        if let Some(p) = self.players.get_mut(&id) {
            p.last_seen_ms = p.last_seen_ms.max(now_ms);
        }
    }

    /// Your HP, as the game sent it. 0 is a death; above it you are up, and
    /// the hits that seemed to kill you were taken by a shield.
    pub fn my_hp(&mut self, hp: i64, now_ms: i64) {
        self.settle(now_ms);
        self.my_hp = Some(hp);
        self.pending_death_ms = None;
        if hp > 0 {
            self.dead = false;
        } else if !self.dead {
            self.dead = true;
            self.died(now_ms);
        }
    }

    /// Anyone hit you for `damage` with `skill`: your HP goes down by it, and
    /// a player in sight is attacking you (`attacked_me`).
    pub fn hit_on_me(&mut self, id: i32, damage: u64, skill: i32, now_ms: i64) {
        self.settle(now_ms);
        if !self.dead && self.pending_death_ms.is_none() {
            if let Some(hp) = self.my_hp.as_mut() {
                *hp -= damage as i64;
                if *hp <= 0 {
                    self.pending_death_ms = Some(now_ms);
                }
            }
        }
        self.attacked_me(id, damage, skill, now_ms);
    }

    /// A death the hits made is confirmed once no HP came for `DEATH_CONFIRM_MS`.
    pub fn settle(&mut self, now_ms: i64) {
        if let Some(at) = self.pending_death_ms.filter(|&at| now_ms - at >= DEATH_CONFIRM_MS) {
            self.pending_death_ms = None;
            self.dead = true;
            self.died(at);
        }
    }

    /// A player hit you for `damage` with `skill`. Only players in sight
    /// count: a mob is not in here.
    pub fn attacked_me(&mut self, id: i32, damage: u64, skill: i32, now_ms: i64) {
        let Some(p) = self.players.get_mut(&id) else { return };
        p.attacked_me_ms = Some(now_ms);
        p.last_seen_ms = p.last_seen_ms.max(now_ms);
        p.taken += damage;
        self.hits_on_me.push_back(HitOnMe { at_ms: now_ms, actor: id, skill, damage });
        while self.hits_on_me.front().is_some_and(|h| now_ms - h.at_ms > RECAP_WINDOW_MS) {
            self.hits_on_me.pop_front();
        }
    }

    /// You hit a player in sight for `damage`.
    pub fn hit_by_me(&mut self, id: i32, damage: u64) {
        if let Some(p) = self.players.get_mut(&id) {
            p.dealt += damage;
        }
    }

    /// You died at `now_ms`: the players' hits of the `RECAP_WINDOW_MS` before
    /// make the death recap (hits after it were on the way already). If no
    /// player hit you (a mob killed you), the last recap stays.
    fn died(&mut self, now_ms: i64) {
        let hits: Vec<_> =
            self.hits_on_me.drain(..).filter(|h| h.at_ms <= now_ms && now_ms - h.at_ms <= RECAP_WINDOW_MS).collect();
        let Some(last) = hits.last() else { return };
        let name_of = |id: i32| self.players.get(&id).map(|p| (p.name.clone(), p.class));
        let Some((killer, _)) = name_of(last.actor) else { return };
        let mut by_actor: HashMap<i32, (u64, u32, HashMap<i32, u64>)> = HashMap::new();
        for h in &hits {
            let entry = by_actor.entry(h.actor).or_default();
            entry.0 += h.damage;
            entry.1 += 1;
            *entry.2.entry(h.skill).or_default() += h.damage;
        }
        let mut attackers: Vec<_> = by_actor
            .into_iter()
            .filter_map(|(actor, (damage, hits, skills))| {
                let (name, class) = name_of(actor)?;
                let top_skill = skills.into_iter().max_by_key(|&(skill, d)| (d, std::cmp::Reverse(skill))).map_or(0, |(s, _)| s);
                Some(RecapAttacker { name, class, damage, hits, top_skill })
            })
            .collect();
        attackers.sort_by(|a, b| b.damage.cmp(&a.damage).then(a.name.cmp(&b.name)));
        self.last_death = Some(DeathRecap { at_ms: now_ms, killer, killing_skill: last.skill, attackers });
    }

    /// The recap of your last death by players' hands.
    pub fn last_death(&mut self, now_ms: i64) -> Option<&DeathRecap> {
        self.settle(now_ms);
        self.last_death.as_ref()
    }

    /// The server took the player out of sight.
    pub fn despawn(&mut self, id: i32) {
        self.players.remove(&id);
    }

    /// A zone load: entity ids start over. The last death recap stays: after
    /// a death you come back up somewhere else.
    pub fn clear(&mut self) {
        self.players.clear();
        self.hits_on_me.clear();
        self.pending_death_ms = None;
    }

    pub fn contains(&self, id: i32) -> bool {
        self.players.contains_key(&id)
    }

    /// The players of the other faction in sight, attacking you first (the
    /// latest attack first), then the one seen last first. None while your
    /// own faction isn't known.
    pub fn enemies(&mut self, now_ms: i64) -> Vec<NearbyPlayer> {
        self.settle(now_ms);
        self.players.retain(|_, p| now_ms - p.last_seen_ms < FORGET_AFTER_MS);
        let Some(mine) = self.self_faction else { return Vec::new() };
        let mut out: Vec<_> = self.players.values().filter(|p| p.faction != mine).cloned().collect();
        out.sort_by(|a, b| {
            b.attacked_me_ms
                .cmp(&a.attacked_me_ms)
                .then(b.last_seen_ms.cmp(&a.last_seen_ms))
                .then(a.id.cmp(&b.id))
        });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_other_faction_attackers_first_then_the_one_seen_last() {
        let mut n = NearbyPlayers::default();
        n.spawn(1, "Friend", None, Some(2200), 2, 0);
        n.spawn(2, "Ranger", Some(JobClass::Ranger), Some(2100), 1, 0);
        n.spawn(3, "HuK", None, None, 1, 0);
        n.spawn(4, "MILFSHAKE", None, None, 1, 0);
        assert!(n.enemies(0).is_empty(), "your faction isn't known yet");
        n.set_self_faction(2);
        n.seen(3, 500);
        n.seen(2, 400);
        n.attacked_me(4, 500, 1, 100);
        let names: Vec<_> = n.enemies(1000).into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["MILFSHAKE", "HuK", "Ranger"]);
    }

    #[test]
    fn a_despawn_or_a_long_silence_ends_a_player_and_a_second_spawn_keeps_the_attack() {
        let mut n = NearbyPlayers::default();
        n.set_self_faction(2);
        n.spawn(2, "Ranger", None, Some(2100), 1, 0);
        n.attacked_me(2, 500, 1, 50);
        n.spawn(2, "Ranger", None, None, 1, 60);
        let ranger = &n.enemies(100)[0];
        assert_eq!((ranger.attacked_me_ms, ranger.item_level, ranger.taken), (Some(50), Some(2100), 500));
        n.despawn(2);
        assert!(n.enemies(100).is_empty());
        n.spawn(3, "HuK", None, None, 1, 0);
        assert!(n.enemies(FORGET_AFTER_MS).is_empty());
    }

    #[test]
    fn a_mob_hitting_you_is_no_player() {
        let mut n = NearbyPlayers::default();
        n.set_self_faction(2);
        n.my_hp(400, 0);
        n.hit_on_me(9, 500, 1, 0);
        assert!(!n.contains(9));
        assert!(n.last_death(5_000).is_none(), "a mob killed you: no recap");
    }

    #[test]
    fn the_damage_between_you_and_them_and_who_killed_you() {
        let mut n = NearbyPlayers::default();
        n.set_self_faction(2);
        n.spawn(1, "Ranger", Some(JobClass::Ranger), None, 1, 0);
        n.spawn(2, "HuK", Some(JobClass::Gladiator), None, 1, 0);
        n.hit_on_me(1, 900, 11, 1_000); // long before: not in the recap
        n.hit_by_me(1, 4_000);
        n.hit_by_me(7, 4_000); // no player in sight
        n.my_hp(5_000, 19_000);
        n.hit_on_me(2, 3_000, 21, 20_000);
        n.hit_on_me(1, 1_000, 12, 21_000);
        n.hit_on_me(1, 1_500, 13, 22_000); // 5 000 HP gone: dead
        n.hit_on_me(2, 500, 22, 22_100); // on its way already
        n.spawn(1, "Ranger", Some(JobClass::Ranger), None, 1, 22_200); // a second copy keeps the damage
        let ranger = n.enemies(22_200).into_iter().find(|p| p.id == 1).unwrap();
        assert_eq!((ranger.dealt, ranger.taken), (4_000, 3_400));
        assert!(n.last_death(23_000).is_none(), "not confirmed yet");

        let recap = n.last_death(23_500).unwrap().clone();
        assert_eq!((recap.at_ms, recap.killer.as_str(), recap.killing_skill), (22_000, "Ranger", 13));
        let rows: Vec<_> = recap.attackers.iter().map(|a| (a.name.as_str(), a.damage, a.hits, a.top_skill)).collect();
        assert_eq!(rows, [("HuK", 3_000, 1, 21), ("Ranger", 2_500, 2, 13)]);

        // Revived; then killed by a mob, the game saying HP 0: no player hit you, so the recap stays.
        n.my_hp(20_000, 40_000);
        n.my_hp(0, 90_000);
        assert_eq!(n.last_death(90_000), Some(&recap));
        n.clear();
        assert_eq!(n.last_death(90_000), Some(&recap), "a zone load keeps it");
    }

    #[test]
    fn hits_a_shield_took_are_no_death() {
        let mut n = NearbyPlayers::default();
        n.set_self_faction(2);
        n.spawn(1, "Ranger", Some(JobClass::Ranger), None, 1, 0);
        n.my_hp(1_000, 0);
        n.hit_on_me(1, 3_000, 11, 100);
        n.my_hp(900, 400); // the HP after: a shield took most of it
        assert!(n.last_death(10_000).is_none());
        n.hit_on_me(1, 2_000, 11, 11_000);
        assert!(n.last_death(12_600).is_some(), "no HP after this one: dead");
    }
}
