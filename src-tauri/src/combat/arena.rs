//! The 1v1 arena (Arena of Solitude): the match against one player, round by
//! round.
//!
//! What a capture of one match showed (2026-11-12, a Gladiator, 3:1): the
//! arena is map 61, loaded again (61 -> 61) between two rounds; both players
//! keep their entity ids through the match. A round ends with one of the two
//! at 0 HP (the `8D` HP records, `DataStorage::note_hp`); before the next
//! one both are back to full and the hits start again. Leaving the map ends
//! the match.
//!
//! A round starts with the first hit between you and the opponent while
//! neither is at 0 HP; it ends with the first 0 HP of either. The opponent is
//! the other team's one player on the roster the game sends as the match
//! starts (`19 e3`, `parse_roster`) — named, with class and server, before
//! the first hit; their spawn (`45 36`) adds the entity and item level — or,
//! without a roster, the first player you hit or who hits you; hits with
//! anyone else are left out. A roster of more
//! than one a side is a team match (5v5 on the same maps): not counted yet.
//!
//! The arena maps are aion2maps.com's instance index's `arena_battlefield`
//! of `ArenaCombat`: 60 Fire Temple Arena, 61 Impetusium Arena (up to 10
//! players each). 62 - 64 are the battlegrounds (`ArenaCar`, `ArenaCTF`,
//! `ArenaDom`, up to 20).

use crate::combat::nearby_players::NearbyPlayer;
use crate::entity::job_class::JobClass;

/// The arena maps (`ArenaCombat`): 1v1 and 5v5 alike. 61 seen in a capture.
pub const SOLO_ARENA_MAPS: [i32; 2] = [60, 61];

/// One player on an arena roster (`19 e3`), team 1 or 2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterEntry {
    pub name: String,
    pub team: u8,
    /// The class (the party roster's class field, `JobClass::from_roster_class`).
    pub class: Option<JobClass>,
    /// Their server: the one after the level, else the home server before the name.
    pub server: Option<u16>,
}

/// The arena roster the game sends as a match starts (`19 e3`), read by the
/// shape of its players: `<id u32> 00 00 <home server u16> <name len u8>
/// <name> <class u32, 1-200> <race u8, 1|2> <level u8> <7 zero bytes>
/// <server u16>`. A team
/// opens with `<team u8> <rating u64> <u32> <count u8> <8 zero bytes>` right
/// before its first player. Seen 2026-11-12: team 1 Bradcrit (class 8, a
/// Gladiator, server 2313), team 2 StayHype (6, a Gladiator, 1303 — the
/// server his spawn named too). None for a body without two players on two
/// teams.
pub fn parse_roster(body: &[u8]) -> Option<Vec<RosterEntry>> {
    let mut out: Vec<RosterEntry> = Vec::new();
    let mut team = 0u8;
    let mut i = 0usize;
    while i + 9 < body.len() {
        let len = body[i + 8] as usize;
        let after = i + 9 + len;
        let shaped = (2..=48).contains(&len)
            && body[i + 4..i + 6] == [0, 0]
            && after + 13 <= body.len()
            && std::str::from_utf8(&body[i + 9..after]).is_ok_and(|n| n.chars().all(|c| !c.is_control()) && n.chars().any(char::is_alphanumeric))
            && (1..=200).contains(&u32::from_le_bytes(body[after..after + 4].try_into().unwrap()))
            && matches!(body[after + 4], 1 | 2)
            && (1..=99).contains(&body[after + 5])
            && body[after + 6..after + 13].iter().all(|&b| b == 0);
        if !shaped {
            i += 1;
            continue;
        }
        if i >= 22 && matches!(body[i - 22], 1 | 2) && body[i - 9] >= 1 && body[i - 8..i].iter().all(|&b| b == 0) {
            team = body[i - 22];
        }
        let name = String::from_utf8_lossy(&body[i + 9..after]).into_owned();
        let class = JobClass::from_roster_class(u32::from_le_bytes(body[after..after + 4].try_into().unwrap()));
        let u16_at = |at: usize| body.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]])).filter(|s| (1..10_000).contains(s));
        let server = u16_at(after + 13).or_else(|| u16_at(i + 6));
        out.push(RosterEntry { name, team: team.max(1), class, server });
        i = after + 13;
    }
    let teams: std::collections::HashSet<u8> = out.iter().map(|e| e.team).collect();
    (out.len() >= 2 && teams.len() == 2).then_some(out)
}

/// The opponent's entity while only the roster named them.
const UNKNOWN_ID: i32 = -1;

/// Matches kept, the newest last: the Companion logs each once it is over.
pub const MATCHES_KEPT: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Me,
    Them,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Round {
    pub started_ms: i64,
    /// None while it is fought.
    pub ended_ms: Option<i64>,
    /// Who won it: the other one went to 0 HP.
    pub winner: Option<Side>,
    /// Your damage on the opponent, and theirs on you.
    pub dealt: u64,
    pub taken: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opponent {
    pub id: i32,
    pub name: String,
    pub class: Option<JobClass>,
    pub item_level: Option<u32>,
    pub server: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArenaMatch {
    pub map: i32,
    pub entered_ms: i64,
    /// When you left the arena: the match is over.
    pub left_ms: Option<i64>,
    pub opponent: Option<Opponent>,
    pub rounds: Vec<Round>,
    /// The roster, once the game sent it.
    pub roster: Option<Vec<RosterEntry>>,
}

impl ArenaMatch {
    /// More than one a side: a team match, not counted (yet).
    pub fn is_team(&self) -> bool {
        self.roster.as_ref().is_some_and(|r| r.len() > 2)
    }

    /// Rounds won by you and by them.
    pub fn score(&self) -> (usize, usize) {
        let won = |side| self.rounds.iter().filter(|r| r.winner == Some(side)).count();
        (won(Side::Me), won(Side::Them))
    }
}

#[derive(Debug, Default)]
pub struct ArenaTracker {
    /// The matches, the newest last; the last one is open while `left_ms` is None.
    matches: Vec<ArenaMatch>,
    /// The last HP of you and the opponent: a round starts only with both up.
    hp: std::collections::HashMap<i32, i64>,
    me: Option<i32>,
}

impl ArenaTracker {
    /// The match being fought, if you are in an arena.
    fn open(&mut self) -> Option<&mut ArenaMatch> {
        self.matches.last_mut().filter(|m| m.left_ms.is_none())
    }

    /// A zone load: into an arena starts a match (the same arena again is
    /// the next round of it), anywhere else ends the open one.
    pub fn map_loaded(&mut self, map: i32, now_ms: i64) {
        if SOLO_ARENA_MAPS.contains(&map) {
            if self.open().is_some_and(|m| m.map == map) {
                return;
            }
            self.close(now_ms);
            self.hp.clear();
            self.matches.push(ArenaMatch { map, entered_ms: now_ms, left_ms: None, opponent: None, rounds: Vec::new(), roster: None });
            while self.matches.len() > MATCHES_KEPT {
                self.matches.remove(0);
            }
        } else {
            self.close(now_ms);
        }
    }

    fn close(&mut self, now_ms: i64) {
        if let Some(m) = self.open() {
            if let Some(round) = m.rounds.last_mut().filter(|r| r.ended_ms.is_none()) {
                // Left in the middle of a round: it has no winner.
                round.ended_ms = Some(now_ms);
            }
            m.left_ms = Some(now_ms);
        }
        self.hp.clear();
    }

    /// The roster as the match starts: in a 1v1 it names the opponent, once
    /// your own name is known (`assign`).
    pub fn roster(&mut self, roster: Vec<RosterEntry>, my_name: Option<&str>) {
        let Some(m) = self.open() else { return };
        m.roster = Some(roster);
        self.assign(my_name);
    }

    /// The roster's other player as the opponent. Without your name nothing:
    /// the roster can come before your self record (2026-11-12: 50 ms
    /// before), and a guess would make you your own opponent. Called again
    /// as the match is read, so it happens once the name is there.
    pub fn assign(&mut self, my_name: Option<&str>) {
        let Some(my_name) = my_name else { return };
        let Some(m) = self.open() else { return };
        let Some(roster) = m.roster.as_ref().filter(|r| r.len() == 2) else { return };
        let mine = roster.iter().find(|e| e.name == my_name).map(|e| e.team);
        let them = roster.iter().find(|e| e.name != my_name && mine.is_none_or(|t| e.team != t)).cloned();
        {
            if let Some(t) = them {
                match &mut m.opponent {
                    // Named by a hit already: what the roster adds.
                    Some(o) if o.name == t.name => {
                        o.class = o.class.or(t.class);
                        o.server = o.server.or(t.server);
                    }
                    // None yet, or the first hit picked someone else: the roster knows
                    // better. Their entity comes with their spawn or first hit.
                    _ => m.opponent = Some(Opponent { id: UNKNOWN_ID, name: t.name.clone(), class: t.class, item_level: None, server: t.server }),
                }
            }
        }
    }

    /// The opponent the roster named whose entity isn't known yet.
    pub fn opponent_without_entity(&self) -> Option<String> {
        self.current()?.opponent.as_ref().filter(|o| o.id == UNKNOWN_ID).map(|o| o.name.clone())
    }

    /// A player spawned in sight: the opponent the roster named gets their
    /// entity, and their item level.
    pub fn player_spawned(&mut self, p: &NearbyPlayer) {
        let Some(o) = self.open().and_then(|m| m.opponent.as_mut()) else { return };
        if o.name != p.name {
            return;
        }
        o.id = p.id;
        o.class = p.class.or(o.class);
        o.item_level = p.item_level.or(o.item_level);
        o.server = o.server.or(p.server);
    }

    /// `actor` hit `target` for `damage`, one of them you. `other` is what is
    /// known of the other one when they are a player (None: not a player).
    pub fn hit(&mut self, me: i32, actor: i32, target: i32, damage: u64, other: Option<&NearbyPlayer>, now_ms: i64) {
        if damage == 0 || actor == target || (actor != me && target != me) {
            return;
        }
        self.me = Some(me);
        let other_id = if actor == me { target } else { actor };
        let hp_up = |id: i32, hp: &std::collections::HashMap<i32, i64>| hp.get(&id).is_none_or(|&v| v > 0);
        let both_up = hp_up(me, &self.hp) && hp_up(other_id, &self.hp);
        let Some(m) = self.open() else { return };
        if m.is_team() {
            return;
        }
        // With a roster, only its players count.
        if let (Some(roster), Some(p)) = (&m.roster, other) {
            if !roster.iter().any(|e| e.name == p.name) {
                return;
            }
        }
        match (&mut m.opponent, other) {
            (None, Some(p)) => {
                m.opponent = Some(Opponent { id: p.id, name: p.name.clone(), class: p.class, item_level: p.item_level, server: p.server });
            }
            (None, None) => return,
            (Some(o), p) if o.id != other_id => {
                // New ids after a load: the same name is the same opponent.
                match p {
                    Some(p) if p.name == o.name => o.id = p.id,
                    _ => return,
                }
            }
            (Some(o), Some(p)) => {
                o.class = o.class.or(p.class);
                o.item_level = p.item_level.or(o.item_level);
                o.server = o.server.or(p.server);
            }
            (Some(_), None) => {}
        }
        if m.rounds.last().is_none_or(|r| r.ended_ms.is_some()) {
            if !both_up {
                return;
            }
            m.rounds.push(Round { started_ms: now_ms, ended_ms: None, winner: None, dealt: 0, taken: 0 });
        }
        let round = m.rounds.last_mut().expect("a round");
        if actor == me {
            round.dealt += damage;
        } else {
            round.taken += damage;
        }
    }

    /// The HP of a player, as the game sent it: 0 for you or the opponent
    /// ends the round, won by the other.
    pub fn hp(&mut self, entity: i32, hp: i64, now_ms: i64) {
        let me = self.me;
        let Some(m) = self.matches.last_mut().filter(|m| m.left_ms.is_none()) else { return };
        let side = if Some(entity) == me {
            Side::Me
        } else if m.opponent.as_ref().is_some_and(|o| o.id == entity) {
            Side::Them
        } else {
            return;
        };
        self.hp.insert(entity, hp);
        if hp > 0 {
            return;
        }
        if let Some(round) = m.rounds.last_mut().filter(|r| r.ended_ms.is_none()) {
            round.ended_ms = Some(now_ms);
            round.winner = Some(if side == Side::Me { Side::Them } else { Side::Me });
        }
    }

    /// The matches kept, the newest last.
    pub fn matches(&self) -> &[ArenaMatch] {
        &self.matches
    }

    /// The match being fought now, if you are in an arena.
    pub fn current(&self) -> Option<&ArenaMatch> {
        self.matches.last().filter(|m| m.left_ms.is_none())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ME: i32 = 14683;
    const OPP: i32 = 12887;

    fn opp() -> NearbyPlayer {
        NearbyPlayer {
            id: OPP,
            name: "StayHype".into(),
            class: Some(JobClass::Gladiator),
            item_level: Some(3100),
            faction: 1,
            paired: false,
            server: Some(2001),
            rank: None,
            last_seen_ms: 0,
            attacked_me_ms: None,
            dealt: 0,
            taken: 0,
        }
    }

    #[test]
    fn the_capture_s_match_comes_out_three_to_one() {
        let o = opp();
        let mut a = ArenaTracker::default();
        a.map_loaded(61, 0);
        // Round 1: lost.
        a.hit(ME, ME, OPP, 1000, Some(&o), 1_000);
        a.hit(ME, OPP, ME, 3000, Some(&o), 2_000);
        a.hp(ME, 0, 3_000);
        // The arena loads again, you are back up; a late hit starts nothing.
        a.map_loaded(61, 4_000);
        a.hit(ME, OPP, ME, 500, Some(&o), 4_100);
        a.hp(ME, 35_685, 5_000);
        // Rounds 2 - 4: won.
        for (i, start) in [10_000, 20_000, 30_000].into_iter().enumerate() {
            a.hit(ME, ME, OPP, 2000, Some(&o), start);
            a.hit(ME, OPP, ME, 100, Some(&o), start + 100);
            a.hp(OPP, 0, start + 1_000);
            a.hp(OPP, 35_264, start + 5_000 + i as i64);
        }
        let m = a.current().expect("open");
        assert_eq!(m.score(), (3, 1));
        assert_eq!(m.rounds.len(), 4);
        assert_eq!(m.rounds[0].dealt, 1000);
        assert_eq!(m.rounds[0].taken, 3000);
        assert_eq!(m.opponent.as_ref().map(|o| o.name.as_str()), Some("StayHype"));
        a.map_loaded(1110, 40_000);
        assert!(a.current().is_none());
        assert_eq!(a.matches()[0].left_ms, Some(40_000));
    }

    #[test]
    fn hits_outside_an_arena_or_with_a_third_player_are_left_out() {
        let o = opp();
        let mut a = ArenaTracker::default();
        a.hit(ME, ME, OPP, 1000, Some(&o), 0);
        assert!(a.matches().is_empty());
        a.map_loaded(61, 0);
        a.hit(ME, ME, OPP, 1000, Some(&o), 1_000);
        let mut other = opp();
        other.id = 999;
        other.name = "Someone".into();
        a.hit(ME, ME, 999, 5000, Some(&other), 1_500);
        a.hit(ME, ME, 777, 5000, None, 1_600);
        assert_eq!(a.current().unwrap().rounds[0].dealt, 1000);
    }

    #[test]
    fn the_capture_s_roster_reads_as_two_teams() {
        let hex = "C1EA020100000016000000000000000800000002011706000000000000170600000100000000000000003B6F02000000090908427261646372697408000000022D000000000000000909170600000000000001170600000000000002000017060000000202060000000000000206000001000000000000000097FD03000000170508537461794879706506000000012D00000000000000170502060000000000000102060000000000000504000206000000";
        let body: Vec<u8> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
        let roster = parse_roster(&body).expect("a roster");
        assert_eq!(
            roster,
            [
                RosterEntry { name: "Bradcrit".into(), team: 1, class: Some(JobClass::Gladiator), server: Some(2313) },
                RosterEntry { name: "StayHype".into(), team: 2, class: Some(JobClass::Gladiator), server: Some(1303) },
            ]
        );
    }

    #[test]
    fn a_team_roster_is_not_counted() {
        let o = opp();
        let mut a = ArenaTracker::default();
        a.map_loaded(60, 0);
        let roster = (0..10).map(|i| RosterEntry { name: format!("P{i}"), team: 1 + (i % 2) as u8, class: None, server: None }).collect();
        a.roster(roster, Some("P0"));
        a.hit(ME, ME, OPP, 1000, Some(&o), 1_000);
        assert!(a.current().unwrap().rounds.is_empty());
    }

    #[test]
    fn the_roster_names_the_opponent_before_the_first_hit() {
        let mut a = ArenaTracker::default();
        a.map_loaded(60, 0);
        let entry = |name: &str, team, class| RosterEntry { name: name.into(), team, class, server: Some(2304) };
        a.roster(vec![entry("Freydis", 1, Some(JobClass::Gladiator)), entry("Bradcrit", 2, Some(JobClass::Gladiator))], Some("Bradcrit"));
        let o = a.current().unwrap().opponent.clone().expect("named by the roster");
        assert_eq!((o.id, o.name.as_str(), o.server), (UNKNOWN_ID, "Freydis", Some(2304)));
        // Their spawn gives the entity and item level; their HP then counts.
        let mut p = opp();
        p.id = 675;
        p.name = "Freydis".into();
        a.player_spawned(&p);
        let o = a.current().unwrap().opponent.clone().unwrap();
        assert_eq!((o.id, o.item_level), (675, Some(3100)));
        a.hit(ME, ME, 675, 1000, Some(&p), 1_000);
        a.hp(675, 0, 2_000);
        assert_eq!(a.current().unwrap().score(), (1, 0));
    }

    #[test]
    fn leaving_mid_round_ends_it_without_a_winner() {
        let o = opp();
        let mut a = ArenaTracker::default();
        a.map_loaded(61, 0);
        a.hit(ME, ME, OPP, 1000, Some(&o), 1_000);
        a.map_loaded(20, 2_000);
        let r = &a.matches()[0].rounds[0];
        assert_eq!((r.ended_ms, r.winner), (Some(2_000), None));
    }
}
