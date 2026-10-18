//! The players around you, for the PvP view: who of the other faction is in
//! sight, and who of them is attacking you.
//!
//! Everything here comes from the server's records about other players: their
//! spawn (`45 36`, with class, faction and item level), their movement and
//! skills (last seen) and their despawn (`47 36`). Where you are is not known
//! (your own movement is only in what the client sends, which is encrypted),
//! so there is no distance: the view orders by attack and by last seen.

use std::collections::HashMap;

use crate::entity::job_class::JobClass;

/// Nothing heard of a player for this long: gone, though no despawn came (it
/// can be missed, mid-capture or in a lost packet).
pub const FORGET_AFTER_MS: i64 = 10 * 60_000;

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
    /// The last time they used a skill on you or hit you.
    pub attacked_me_ms: Option<i64>,
}

#[derive(Default)]
pub struct NearbyPlayers {
    players: HashMap<i32, NearbyPlayer>,
    self_faction: Option<u8>,
}

impl NearbyPlayers {
    /// Your faction, from your self record.
    pub fn set_self_faction(&mut self, faction: u8) {
        self.self_faction = Some(faction);
    }

    /// A player spawned in sight. The same entity again (a second copy of
    /// the record) keeps when it last attacked you.
    pub fn spawn(&mut self, id: i32, name: &str, class: Option<JobClass>, item_level: Option<u32>, faction: u8, now_ms: i64) {
        let attacked_me_ms = self.players.get(&id).filter(|p| p.name == name).and_then(|p| p.attacked_me_ms);
        let item_level = item_level.or_else(|| self.players.get(&id).filter(|p| p.name == name).and_then(|p| p.item_level));
        self.players.insert(
            id,
            NearbyPlayer { id, name: name.to_string(), class, item_level, faction, last_seen_ms: now_ms, attacked_me_ms },
        );
    }

    /// A player moved or used a skill.
    pub fn seen(&mut self, id: i32, now_ms: i64) {
        if let Some(p) = self.players.get_mut(&id) {
            p.last_seen_ms = p.last_seen_ms.max(now_ms);
        }
    }

    /// A player used a skill on you or hit you. Only players in sight count:
    /// a mob is not in here.
    pub fn attacked_me(&mut self, id: i32, now_ms: i64) {
        if let Some(p) = self.players.get_mut(&id) {
            p.attacked_me_ms = Some(now_ms);
            p.last_seen_ms = p.last_seen_ms.max(now_ms);
        }
    }

    /// The server took the player out of sight.
    pub fn despawn(&mut self, id: i32) {
        self.players.remove(&id);
    }

    /// A zone load: entity ids start over.
    pub fn clear(&mut self) {
        self.players.clear();
    }

    pub fn contains(&self, id: i32) -> bool {
        self.players.contains_key(&id)
    }

    /// The players of the other faction in sight, attacking you first (the
    /// latest attack first), then the one seen last first. None while your
    /// own faction isn't known.
    pub fn enemies(&mut self, now_ms: i64) -> Vec<NearbyPlayer> {
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
        n.attacked_me(4, 100);
        let names: Vec<_> = n.enemies(1000).into_iter().map(|p| p.name).collect();
        assert_eq!(names, ["MILFSHAKE", "HuK", "Ranger"]);
    }

    #[test]
    fn a_despawn_or_a_long_silence_ends_a_player_and_a_second_spawn_keeps_the_attack() {
        let mut n = NearbyPlayers::default();
        n.set_self_faction(2);
        n.spawn(2, "Ranger", None, Some(2100), 1, 0);
        n.attacked_me(2, 50);
        n.spawn(2, "Ranger", None, None, 1, 60);
        let ranger = &n.enemies(100)[0];
        assert_eq!((ranger.attacked_me_ms, ranger.item_level), (Some(50), Some(2100)));
        n.despawn(2);
        assert!(n.enemies(100).is_empty());
        n.spawn(3, "HuK", None, None, 1, 0);
        assert!(n.enemies(FORGET_AFTER_MS).is_empty());
    }

    #[test]
    fn a_mob_hitting_you_is_no_player() {
        let mut n = NearbyPlayers::default();
        n.set_self_faction(2);
        n.attacked_me(9, 0);
        assert!(!n.contains(9));
    }
}
