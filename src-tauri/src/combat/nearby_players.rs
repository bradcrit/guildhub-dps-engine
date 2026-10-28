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
//! append_damage`): what you dealt each, what each dealt you. A "hit" from
//! one player on another is a heal or a buff just as often — the game sends
//! both alike, and the meter takes every one between players for healing —
//! so only one between the two factions counts here; between players of one
//! faction, or with a faction not known, it is left out (a party Cleric's
//! Radiant Recovery was taken for the hits that killed you, in a dungeon on
//! 2026-10-18). The hits of the
//! last `RECAP_WINDOW_MS` on you and on each player in sight are kept, so that
//! when you die the death recap can say who killed you, with what — and when
//! an enemy you hit dies, it is your kill (the last hit yours) or assist.
//!
//! The game sends no HP 0 when you die to players: your last HP was 625, then
//! hits came and nothing more until your revive 24 s later (Bradcrit in the
//! Abyss, 17:05:32 on 2026-10-18). So a death is the last HP minus the hits
//! after it reaching 0, once no HP came within `DEATH_CONFIRM_MS` to say a
//! shield took them (`Vitals`) — yours and, the same way, an enemy's (not yet
//! seen in a capture: none of 2026-10-18 has a player killed by you).

use std::collections::{HashMap, VecDeque};

use crate::entity::job_class::JobClass;

/// The Abyss: map 20 in the map loads (`21 36`) of both trips there on
/// 2026-10-18 (01:29 and 17:04), the home map 1110 before and after.
pub const ABYSS_MAPS: [i32; 1] = [20];

/// Nothing heard of a player for this long: gone, though no despawn came (it
/// can be missed, mid-capture or in a lost packet).
pub const FORGET_AFTER_MS: i64 = 10 * 60_000;

/// A death recap, or a kill, counts the hits of this long before the death.
pub const RECAP_WINDOW_MS: i64 = 15_000;

/// The hits since the last HP came to more than it: dead, unless a new HP
/// comes within this long (a shield took them). Your HP came every 100-400 ms
/// while you were hit.
pub const DEATH_CONFIRM_MS: i64 = 1_500;

/// Kills kept for the event: the Companion logs each once.
pub const KILLS_KEPT: usize = 50;

#[derive(Debug, Clone, PartialEq)]
pub struct NearbyPlayer {
    pub id: i32,
    pub name: String,
    pub class: Option<JobClass>,
    /// Equipment item level, as the inspect window shows it.
    pub item_level: Option<u32>,
    /// The spawn's faction byte; yours is in your self record. In an Abyss
    /// pairing of one race, the side byte instead (`paired`).
    pub faction: u8,
    /// `faction` is a side of an Abyss pairing of one race, not the race:
    /// yours then is a party member's (`guess_faction_from_party`).
    pub paired: bool,
    /// Their server, from their legion's entry in the spawn; None out of one.
    pub server: Option<u16>,
    /// The Abyss rank: 19 Trainee, 18 Ninth-Class … 10 First-Class Soldier.
    pub rank: Option<u8>,
    pub last_seen_ms: i64,
    /// The last time they hit you.
    pub attacked_me_ms: Option<i64>,
    /// Your damage on them since they came into sight.
    pub dealt: u64,
    /// Their damage on you since they came into sight.
    pub taken: u64,
}

/// One hit, on you or on a player in sight.
#[derive(Debug, Clone, PartialEq)]
struct Hit {
    at_ms: i64,
    actor: i32,
    skill: i32,
    damage: u64,
}

/// An HP the game sent, less the hits since. A death at an HP of 0, or once
/// the hits took it to 0 and no HP came within `DEATH_CONFIRM_MS`. Each call
/// returns the time of a death it found.
#[derive(Debug, Clone, Default, PartialEq)]
struct Vitals {
    hp: Option<i64>,
    /// When the hits took `hp` to 0, until confirmed or a new HP came.
    pending_ms: Option<i64>,
    /// Down, until an HP above 0 says they are back up.
    dead: bool,
}

impl Vitals {
    fn report(&mut self, hp: i64, now_ms: i64) -> Option<i64> {
        let settled = self.settle(now_ms);
        self.hp = Some(hp);
        self.pending_ms = None;
        if hp > 0 {
            self.dead = false;
        } else if !self.dead {
            self.dead = true;
            return Some(now_ms);
        }
        settled
    }

    fn hit(&mut self, damage: u64, now_ms: i64) -> Option<i64> {
        let settled = self.settle(now_ms);
        if !self.dead && self.pending_ms.is_none() {
            if let Some(hp) = self.hp.as_mut() {
                *hp -= damage as i64;
                if *hp <= 0 {
                    self.pending_ms = Some(now_ms);
                }
            }
        }
        settled
    }

    fn settle(&mut self, now_ms: i64) -> Option<i64> {
        let at = self.pending_ms.filter(|&at| now_ms - at >= DEATH_CONFIRM_MS)?;
        self.pending_ms = None;
        self.dead = true;
        Some(at)
    }

    /// Out of sight right after the hits took them to 0: that was a death.
    fn confirm(&mut self) -> Option<i64> {
        let at = self.pending_ms.take()?;
        self.dead = true;
        Some(at)
    }
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
    /// You died in the Abyss.
    pub abyss: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RecapAttacker {
    pub name: String,
    pub class: Option<JobClass>,
    pub rank: Option<u8>,
    pub damage: u64,
    pub hits: u32,
    /// The skill that did the most of their damage.
    pub top_skill: i32,
}

/// An enemy player who died with your hits on them in the `RECAP_WINDOW_MS` before.
#[derive(Debug, Clone, PartialEq)]
pub struct KillRecord {
    pub at_ms: i64,
    pub name: String,
    pub class: Option<JobClass>,
    pub item_level: Option<u32>,
    pub rank: Option<u8>,
    /// The last hit was yours: a kill; else an assist.
    pub killing_blow: bool,
    /// Your damage on them, your hits and the skill that did the most of it.
    pub damage: u64,
    pub hits: u32,
    pub top_skill: i32,
    /// Everyone's damage on them, and how many besides you hit them.
    pub total: u64,
    pub helpers: u32,
    pub abyss: bool,
}

/// What a player in sight has taken: their HP and the recent hits on them.
#[derive(Debug, Clone, Default)]
struct Foe {
    vitals: Vitals,
    hits: VecDeque<Hit>,
}

#[derive(Default)]
pub struct NearbyPlayers {
    players: HashMap<i32, NearbyPlayer>,
    foes: HashMap<i32, Foe>,
    self_faction: Option<u8>,
    /// Your side in an Abyss pairing of one race, from a party member in
    /// sight; until known, a player of your server's (`mine`).
    self_side: Option<u8>,
    /// Your server, from your self record.
    self_server: Option<u16>,
    /// Your entity, from the hits (`DataStorage::note_nearby_damage`).
    me: Option<i32>,
    abyss: bool,
    hits_on_me: VecDeque<Hit>,
    my_vitals: Vitals,
    last_death: Option<DeathRecap>,
    kills: VecDeque<KillRecord>,
}

fn push_hit(hits: &mut VecDeque<Hit>, hit: Hit) {
    let now = hit.at_ms;
    hits.push_back(hit);
    while hits.front().is_some_and(|h| now - h.at_ms > RECAP_WINDOW_MS) {
        hits.pop_front();
    }
}

/// The hits of the `RECAP_WINDOW_MS` up to a death at `at` (later ones were on the way already).
fn hits_before(hits: &mut VecDeque<Hit>, at: i64) -> Vec<Hit> {
    hits.drain(..).filter(|h| h.at_ms <= at && at - h.at_ms <= RECAP_WINDOW_MS).collect()
}

/// The skill with the most damage among `hits`.
fn top_skill<'a>(hits: impl Iterator<Item = &'a Hit>) -> i32 {
    let mut by_skill: HashMap<i32, u64> = HashMap::new();
    for h in hits {
        *by_skill.entry(h.skill).or_default() += h.damage;
    }
    by_skill.into_iter().max_by_key(|&(skill, d)| (d, std::cmp::Reverse(skill))).map_or(0, |(s, _)| s)
}

impl NearbyPlayers {
    /// Your faction, from your self record. It comes with a zone load, after
    /// which the side of a pairing is learned again.
    pub fn set_self_faction(&mut self, faction: u8) {
        self.self_faction = Some(faction);
        self.self_side = None;
    }

    pub fn self_faction(&self) -> Option<u8> {
        self.self_faction
    }

    /// Your server, from your self record.
    pub fn set_self_server(&mut self, server: u16) {
        self.self_server = Some(server);
    }

    /// Your faction or side, as the players in sight are compared with it:
    /// in a pairing of one race a party member's side, else the side of a
    /// player in sight whose legion is of your server (solo, 2026-10-28:
    /// 45 of them on Bradcrit's server 2313 all on side 2, the 4 of 2311 on
    /// side 1), else your self record's faction.
    fn mine(&self) -> Option<u8> {
        let server_side = || {
            let server = self.self_server?;
            self.players.values().find(|p| p.paired && p.server == Some(server)).map(|p| p.faction)
        };
        self.self_side.or_else(server_side).or(self.self_faction)
    }

    /// A player in sight of the other faction — false while your own isn't known.
    pub fn is_enemy(&self, id: i32) -> bool {
        self.mine().is_some_and(|mine| self.players.get(&id).is_some_and(|p| p.faction != mine))
    }

    /// Your faction until your self record says it — the one you had last
    /// time, or a party member's in sight. The record comes only with a zone
    /// load: a meter started in the Abyss (21:24 on 2026-10-18) had none until
    /// you left, so it saw no enemies and none of your two kills there.
    pub fn guess_self_faction(&mut self, faction: u8) {
        if self.self_faction.is_none() && matches!(faction, 1 | 2) {
            self.self_faction = Some(faction);
        }
    }

    /// `guess_self_faction` from the first player in sight `in_party` says is
    /// in your party — and in a pairing of one race your side, which your
    /// self record doesn't state (it said 2, the race, on 2026-10-28).
    pub fn guess_faction_from_party(&mut self, in_party: impl Fn(&NearbyPlayer) -> bool) {
        if self.self_faction.is_none() {
            if let Some(faction) = self.players.values().find(|p| in_party(p)).map(|p| p.faction) {
                self.guess_self_faction(faction);
            }
        }
        if self.self_side.is_none() {
            self.self_side = self.players.values().find(|p| p.paired && in_party(p)).map(|p| p.faction);
        }
    }

    /// Whether a "hit" between players of these factions is one — not a heal
    /// or a buff between players of one faction.
    fn across_factions(a: Option<u8>, b: Option<u8>) -> bool {
        matches!((a, b), (Some(a), Some(b)) if a != b)
    }

    /// Your entity id.
    pub fn set_me(&mut self, id: i32) {
        self.me = Some(id);
    }

    /// Whether the map you are on is the Abyss — kills and deaths say it.
    pub fn set_abyss(&mut self, abyss: bool) {
        self.abyss = abyss;
    }

    /// A player spawned in sight. The same entity again (a second copy of
    /// the record) keeps when it last attacked you and the damage between you.
    pub fn spawn(&mut self, id: i32, name: &str, class: Option<JobClass>, item_level: Option<u32>, faction: u8, paired: bool, now_ms: i64) {
        let same = self.players.get(&id).filter(|p| p.name == name);
        let attacked_me_ms = same.and_then(|p| p.attacked_me_ms);
        let item_level = item_level.or_else(|| same.and_then(|p| p.item_level));
        let rank = same.and_then(|p| p.rank);
        let server = same.and_then(|p| p.server);
        let (dealt, taken) = same.map_or((0, 0), |p| (p.dealt, p.taken));
        if same.is_none() {
            self.foes.remove(&id);
        }
        self.players.insert(
            id,
            NearbyPlayer {
                id,
                name: name.to_string(),
                class,
                item_level,
                faction,
                paired,
                server,
                rank,
                last_seen_ms: now_ms,
                attacked_me_ms,
                dealt,
                taken,
            },
        );
    }

    /// A player's server, as their spawn told it (kept when it didn't).
    pub fn set_server(&mut self, id: i32, server: Option<u16>) {
        if let (Some(p), Some(server)) = (self.players.get_mut(&id), server) {
            p.server = Some(server);
        }
    }

    /// A player's Abyss rank, as their spawn told it (kept when it didn't).
    pub fn set_rank(&mut self, id: i32, rank: Option<u8>) {
        if let (Some(p), Some(rank)) = (self.players.get_mut(&id), rank) {
            p.rank = Some(rank);
        }
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
        if let Some(at) = self.my_vitals.report(hp, now_ms) {
            self.died(at);
        }
    }

    /// A player in sight's HP, as the game sent it.
    pub fn player_hp(&mut self, id: i32, hp: i64, now_ms: i64) {
        self.settle(now_ms);
        if !self.players.contains_key(&id) {
            return;
        }
        if let Some(at) = self.foes.entry(id).or_default().vitals.report(hp, now_ms) {
            self.player_died(id, at);
        }
    }

    /// `actor` hit you for `damage` with `skill` — a mob, or a player
    /// (`actor_is_player`): your HP goes down by it, and an enemy player in
    /// sight is attacking you (`attacked_me`). From a player of your faction,
    /// one not in sight or while your faction isn't known, it's left out.
    pub fn hit_on_me(&mut self, actor: i32, damage: u64, skill: i32, actor_is_player: bool, now_ms: i64) {
        self.settle(now_ms);
        let from_player = actor_is_player || self.players.contains_key(&actor);
        if from_player && !Self::across_factions(self.mine(), self.players.get(&actor).map(|p| p.faction)) {
            return;
        }
        if let Some(at) = self.my_vitals.hit(damage, now_ms) {
            self.died(at);
        }
        self.attacked_me(actor, damage, skill, now_ms);
    }

    /// `actor` hit a player in sight for `damage` with `skill`: their HP goes
    /// down by it, and when `actor` is you, it is your damage on them. From a
    /// player of their own faction (a heal, a buff) or one whose faction
    /// isn't known, it's left out.
    pub fn hit_on_player(&mut self, id: i32, actor: i32, damage: u64, skill: i32, actor_is_player: bool, now_ms: i64) {
        self.settle(now_ms);
        let Some(target_faction) = self.players.get(&id).map(|p| p.faction) else { return };
        let mine = Some(actor) == self.me;
        let actor_faction = if mine { self.mine() } else { self.players.get(&actor).map(|p| p.faction) };
        let from_player = mine || actor_is_player || actor_faction.is_some();
        if from_player && !Self::across_factions(actor_faction, Some(target_faction)) {
            return;
        }
        if mine {
            if let Some(p) = self.players.get_mut(&id) {
                p.dealt += damage;
            }
        }
        let foe = self.foes.entry(id).or_default();
        if from_player {
            push_hit(&mut foe.hits, Hit { at_ms: now_ms, actor, skill, damage });
        }
        if let Some(at) = foe.vitals.hit(damage, now_ms) {
            self.player_died(id, at);
        }
    }

    /// Deaths the hits made are confirmed once no HP came for `DEATH_CONFIRM_MS`.
    pub fn settle(&mut self, now_ms: i64) {
        if let Some(at) = self.my_vitals.settle(now_ms) {
            self.died(at);
        }
        let deaths: Vec<_> = self.foes.iter_mut().filter_map(|(&id, foe)| foe.vitals.settle(now_ms).map(|at| (id, at))).collect();
        for (id, at) in deaths {
            self.player_died(id, at);
        }
    }

    /// A player hit you for `damage` with `skill`. Only players in sight
    /// count: a mob is not in here.
    pub fn attacked_me(&mut self, id: i32, damage: u64, skill: i32, now_ms: i64) {
        let Some(p) = self.players.get_mut(&id) else { return };
        p.attacked_me_ms = Some(now_ms);
        p.last_seen_ms = p.last_seen_ms.max(now_ms);
        p.taken += damage;
        push_hit(&mut self.hits_on_me, Hit { at_ms: now_ms, actor: id, skill, damage });
    }

    /// You died at `at`: the players' hits of the `RECAP_WINDOW_MS` before
    /// make the death recap. If no player hit you (a mob killed you), the
    /// last recap stays.
    fn died(&mut self, at: i64) {
        let hits = hits_before(&mut self.hits_on_me, at);
        let Some(last) = hits.last() else { return };
        let name_of = |id: i32| self.players.get(&id).map(|p| (p.name.clone(), p.class, p.rank));
        let Some((killer, ..)) = name_of(last.actor) else { return };
        let mut actors: Vec<i32> = hits.iter().map(|h| h.actor).collect();
        actors.sort_unstable();
        actors.dedup();
        let mut attackers: Vec<_> = actors
            .into_iter()
            .filter_map(|actor| {
                let (name, class, rank) = name_of(actor)?;
                let theirs = || hits.iter().filter(move |h| h.actor == actor);
                let damage = theirs().map(|h| h.damage).sum();
                Some(RecapAttacker { name, class, rank, damage, hits: theirs().count() as u32, top_skill: top_skill(theirs()) })
            })
            .collect();
        attackers.sort_by(|a, b| b.damage.cmp(&a.damage).then(a.name.cmp(&b.name)));
        self.last_death = Some(DeathRecap { at_ms: at, killer, killing_skill: last.skill, attackers, abyss: self.abyss });
    }

    /// A player in sight died at `at`: your kill, or assist, if they are of
    /// the other faction and you hit them in the `RECAP_WINDOW_MS` before.
    fn player_died(&mut self, id: i32, at: i64) {
        let (Some(me), Some(mine)) = (self.me, self.mine()) else { return };
        let Some(p) = self.players.get(&id).filter(|p| p.faction != mine) else { return };
        let Some(foe) = self.foes.get_mut(&id) else { return };
        let hits = hits_before(&mut foe.hits, at);
        let my_hits: Vec<_> = hits.iter().filter(|h| h.actor == me).collect();
        if my_hits.is_empty() {
            return;
        }
        let mut helpers: Vec<i32> = hits.iter().map(|h| h.actor).filter(|&a| a != me).collect();
        helpers.sort_unstable();
        helpers.dedup();
        self.kills.push_back(KillRecord {
            at_ms: at,
            name: p.name.clone(),
            class: p.class,
            item_level: p.item_level,
            rank: p.rank,
            killing_blow: hits.last().is_some_and(|h| h.actor == me),
            damage: my_hits.iter().map(|h| h.damage).sum(),
            hits: my_hits.len() as u32,
            top_skill: top_skill(my_hits.iter().copied()),
            total: hits.iter().map(|h| h.damage).sum(),
            helpers: helpers.len() as u32,
            abyss: self.abyss,
        });
        while self.kills.len() > KILLS_KEPT {
            self.kills.pop_front();
        }
    }

    /// The recap of your last death by players' hands.
    pub fn last_death(&mut self, now_ms: i64) -> Option<&DeathRecap> {
        self.settle(now_ms);
        self.last_death.as_ref()
    }

    /// Your last `KILLS_KEPT` kills and assists, oldest first.
    pub fn kills(&mut self, now_ms: i64) -> Vec<KillRecord> {
        self.settle(now_ms);
        self.kills.iter().cloned().collect()
    }

    /// The server took the player out of sight — right after the hits took
    /// them to 0, that was their death.
    pub fn despawn(&mut self, id: i32) {
        if let Some(at) = self.foes.get_mut(&id).and_then(|f| f.vitals.confirm()) {
            self.player_died(id, at);
        }
        self.players.remove(&id);
        self.foes.remove(&id);
    }

    /// A zone load: entity ids start over. The last death recap and the
    /// kills stay: after a death you come back up somewhere else.
    pub fn clear(&mut self) {
        self.players.clear();
        self.foes.clear();
        self.hits_on_me.clear();
        self.my_vitals.pending_ms = None;
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
        let players = &self.players;
        self.foes.retain(|id, _| players.contains_key(id));
        let Some(mine) = self.mine() else { return Vec::new() };
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
        n.spawn(1, "Friend", None, Some(2200), 2, false, 0);
        n.spawn(2, "Ranger", Some(JobClass::Ranger), Some(2100), 1, false, 0);
        n.spawn(3, "HuK", None, None, 1, false, 0);
        n.spawn(4, "MILFSHAKE", None, None, 1, false, 0);
        assert!(n.enemies(0).is_empty(), "your faction isn't known yet");
        n.set_self_faction(2);
        n.seen(3, 500);
        n.seen(2, 400);
        n.attacked_me(4, 500, 1, 100);
        let names: Vec<_> = n.enemies(1000).into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["MILFSHAKE", "HuK", "Ranger"]);
    }

    #[test]
    fn an_enemy_is_a_player_in_sight_of_the_other_faction() {
        let mut n = NearbyPlayers::default();
        n.spawn(1, "Friend", None, None, 2, false, 0);
        n.spawn(2, "Foe", None, None, 1, false, 0);
        assert!(!n.is_enemy(2), "your faction isn't known yet");
        n.set_self_faction(2);
        assert!(!n.is_enemy(1));
        assert!(n.is_enemy(2));
        assert!(!n.is_enemy(3), "not in sight");
    }

    #[test]
    fn a_despawn_or_a_long_silence_ends_a_player_and_a_second_spawn_keeps_the_attack() {
        let mut n = NearbyPlayers::default();
        n.set_self_faction(2);
        n.spawn(2, "Ranger", None, Some(2100), 1, false, 0);
        n.attacked_me(2, 500, 1, 50);
        n.spawn(2, "Ranger", None, None, 1, false, 60);
        let ranger = &n.enemies(100)[0];
        assert_eq!((ranger.attacked_me_ms, ranger.item_level, ranger.taken), (Some(50), Some(2100), 500));
        n.despawn(2);
        assert!(n.enemies(100).is_empty());
        n.spawn(3, "HuK", None, None, 1, false, 0);
        assert!(n.enemies(FORGET_AFTER_MS).is_empty());
    }

    #[test]
    fn a_mob_hitting_you_is_no_player() {
        let mut n = NearbyPlayers::default();
        n.set_self_faction(2);
        n.my_hp(400, 0);
        n.hit_on_me(9, 500, 1, false, 0);
        assert!(!n.contains(9));
        assert!(n.last_death(5_000).is_none(), "a mob killed you: no recap");
    }

    #[test]
    fn the_damage_between_you_and_them_and_who_killed_you() {
        let mut n = NearbyPlayers::default();
        n.set_self_faction(2);
        n.set_me(100);
        n.set_abyss(true);
        n.spawn(1, "Ranger", Some(JobClass::Ranger), None, 1, false, 0);
        n.spawn(2, "HuK", Some(JobClass::Gladiator), None, 1, false, 0);
        n.hit_on_me(1, 900, 11, true, 1_000); // long before: not in the recap
        n.hit_on_player(1, 100, 4_000, 31, true, 1_000);
        n.hit_on_player(7, 100, 4_000, 31, true, 1_000); // no player in sight
        n.my_hp(5_000, 19_000);
        n.hit_on_me(2, 3_000, 21, true, 20_000);
        n.hit_on_me(1, 1_000, 12, true, 21_000);
        n.hit_on_me(1, 1_500, 13, true, 22_000); // 5 000 HP gone: dead
        n.hit_on_me(2, 500, 22, true, 22_100); // on its way already
        n.spawn(1, "Ranger", Some(JobClass::Ranger), None, 1, false, 22_200); // a second copy keeps the damage
        let ranger = n.enemies(22_200).into_iter().find(|p| p.id == 1).unwrap();
        assert_eq!((ranger.dealt, ranger.taken), (4_000, 3_400));
        assert!(n.last_death(23_000).is_none(), "not confirmed yet");

        let recap = n.last_death(23_500).unwrap().clone();
        assert_eq!((recap.at_ms, recap.killer.as_str(), recap.killing_skill, recap.abyss), (22_000, "Ranger", 13, true));
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
        n.spawn(1, "Ranger", Some(JobClass::Ranger), None, 1, false, 0);
        n.my_hp(1_000, 0);
        n.hit_on_me(1, 3_000, 11, true, 100);
        n.my_hp(900, 400); // the HP after: a shield took most of it
        assert!(n.last_death(10_000).is_none());
        n.hit_on_me(1, 2_000, 11, true, 11_000);
        assert!(n.last_death(12_600).is_some(), "no HP after this one: dead");
    }

    #[test]
    fn an_enemy_you_hit_dies_your_kill_or_your_assist() {
        let mut n = NearbyPlayers::default();
        n.set_self_faction(2);
        n.set_me(100);
        n.spawn(1, "Ranger", Some(JobClass::Ranger), Some(2300), 1, false, 0);
        n.spawn(2, "HuK", None, None, 1, false, 0);
        n.spawn(3, "Friend", None, None, 2, false, 0);
        n.spawn(4, "Stranger", None, None, 1, false, 0);
        n.spawn(200, "Mate", None, None, 2, false, 0);

        // Ranger: HP 0 after your last hit, a party member's before it.
        n.player_hp(1, 5_000, 1_000);
        n.hit_on_player(1, 200, 2_000, 41, true, 2_000);
        n.hit_on_player(1, 100, 1_000, 31, true, 2_100);
        n.hit_on_player(1, 100, 2_500, 32, true, 2_200);
        n.player_hp(1, 0, 2_250);
        // HuK: your hits, then another's; out of sight, with no HP: dead.
        n.player_hp(2, 3_000, 3_000);
        n.hit_on_player(2, 100, 2_000, 31, true, 3_100);
        n.hit_on_player(2, 200, 1_500, 41, true, 3_200);
        n.despawn(2);
        // Your own faction, and one you never hit: no kill.
        n.player_hp(3, 1_000, 4_000);
        n.hit_on_player(3, 100, 2_000, 31, true, 4_100);
        n.player_hp(4, 1_000, 4_000);
        n.hit_on_player(4, 200, 2_000, 41, true, 4_100);

        let kills = n.kills(10_000);
        let rows: Vec<_> =
            kills.iter().map(|k| (k.name.as_str(), k.at_ms, k.killing_blow, k.damage, k.hits, k.top_skill, k.total, k.helpers)).collect();
        assert_eq!(rows, [("Ranger", 2_250, true, 3_500, 2, 32, 5_500, 1), ("HuK", 3_200, false, 2_000, 1, 31, 3_500, 1)]);
        assert_eq!(kills[0].item_level, Some(2300));
    }

    #[test]
    fn a_heal_from_your_own_faction_is_no_hit() {
        // A party Cleric's Radiant Recovery comes as a "hit" on you, as on a
        // party member: no attack, no HP lost, no death recap.
        let mut n = NearbyPlayers::default();
        n.set_self_faction(2);
        n.set_me(100);
        n.spawn(1, "Mirajane23", Some(JobClass::Cleric), None, 2, false, 0);
        n.spawn(2, "Okk", None, None, 2, false, 0);
        n.my_hp(5_000, 0);
        n.hit_on_me(1, 7_640, 41, true, 100);
        n.hit_on_me(77, 9_000, 41, true, 100); // a player not in sight: can't tell
        n.player_hp(2, 1_000, 0);
        n.hit_on_player(2, 1, 3_000, 41, true, 100);
        n.hit_on_player(2, 100, 3_000, 31, true, 100);
        assert!(n.last_death(5_000).is_none());
        assert!(n.kills(5_000).is_empty());
        assert!(n.players.values().all(|p| p.attacked_me_ms.is_none() && p.dealt == 0));
    }

    #[test]
    fn your_faction_not_known_yet_counts_no_hit_and_a_party_member_tells_it() {
        // No self record since the meter started (it comes with a zone load).
        let mut n = NearbyPlayers::default();
        n.set_me(100);
        n.spawn(1, "Ranger", None, None, 1, false, 0);
        n.spawn(3, "Mate", None, None, 2, false, 0);
        n.player_hp(1, 1_000, 0);
        n.hit_on_player(1, 100, 500, 31, true, 100);
        assert!(n.enemies(200).is_empty());
        assert_eq!(n.players[&1].dealt, 0, "a heal for all it knows");
        n.guess_faction_from_party(|p| p.name == "Mate");
        assert_eq!(n.self_faction(), Some(2));
        n.hit_on_player(1, 100, 1_500, 31, true, 300);
        let names: Vec<_> = n.enemies(400).into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["Ranger"]);
        assert_eq!(n.kills(2_000).len(), 1, "and the kill counts");
        n.set_self_faction(1); // the self record has the last word
        assert_eq!(n.self_faction(), Some(1));
    }

    #[test]
    fn in_a_pairing_of_one_race_the_side_tells_friend_from_foe() {
        // Asmodians against Asmodians (2026-10-28): your self record says 2,
        // the race; the spawns say the side.
        let mut n = NearbyPlayers::default();
        n.set_self_faction(2);
        n.spawn(1, "Arvein", None, None, 1, true, 0);
        n.spawn(2, "Lilin", None, None, 2, true, 0);
        let names: Vec<_> = n.enemies(100).into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["Arvein"]);
        // Your server on side 1: a party member in sight says so.
        n.spawn(3, "Mate", None, None, 1, true, 0);
        n.guess_faction_from_party(|p| p.name == "Mate");
        let names: Vec<_> = n.enemies(100).into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["Lilin"]);
        // A zone load: learned again.
        n.set_self_faction(2);
        assert!(n.is_enemy(1));
    }

    #[test]
    fn solo_in_a_pairing_of_one_race_a_legion_of_your_server_tells_your_side() {
        // On server 2311, whose side is 1, solo: the self record's 2 is the race.
        let mut n = NearbyPlayers::default();
        n.set_self_faction(2);
        n.set_self_server(2311);
        n.spawn(1, "Raly", None, None, 1, true, 0);
        n.spawn(2, "Kronk", None, None, 2, true, 0);
        // No one of your server in sight: the self record's 2, wrong here.
        n.set_server(2, Some(2313));
        let names: Vec<_> = n.enemies(100).into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["Raly"]);
        // Raly's legion is of your server: your side is his.
        n.set_server(1, Some(2311));
        let names: Vec<_> = n.enemies(100).into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["Kronk"]);
        // A party member in sight says it before them.
        n.spawn(3, "Mate", None, None, 2, true, 0);
        n.guess_faction_from_party(|p| p.name == "Mate");
        let names: Vec<_> = n.enemies(100).into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["Raly"]);
    }
}
