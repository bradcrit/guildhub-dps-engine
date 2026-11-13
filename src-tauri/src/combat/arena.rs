//! The arena: a 1v1 match against one player, round by round, or a 5v5
//! between two teams.
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
//! anyone else are left out. First to three rounds wins (3:1, 3:0, 2:3 in the
//! captures of 2026-11-12).
//!
//! A roster of more than one a side is a team match (5v5 on the same maps):
//! every player of the roster with their damage on the other team, the
//! damage they took from it and their deaths; a round ends when one team is
//! all at 0 HP. One round decided the 5v5 of 2026-11-12 (yours wiped in 18 s,
//! then out of the arena). Players get their entity from their spawn, you
//! from your own hits.
//!
//! The arena maps are aion2maps.com's instance index's `arena_battlefield`
//! of `ArenaCombat`: 60 Fire Temple Arena, 61 Impetusium Arena (up to 10
//! players each). 62 - 64 are the battlegrounds (`ArenaCar`, `ArenaCTF`,
//! `ArenaDom`, up to 20).
//!
//! A battleground (10v10, map 62 "Arena of Tactics" in the capture of
//! 2026-11-13) is a team match without rounds: ten minutes on a clock, you
//! respawn (the map loads again, 62 -> 62, after each of your deaths), and
//! the game keeps the score itself. What it sends, all checked against its
//! end screen (Victory 85:50, every player's kills, deaths and assists):
//! - `26 e3` (`parse_stats`): the kills, deaths and assists of the players a
//!   kill changed — killer, victim, assisters — by their roster key. Also in
//!   the 5v5. A team's score is the other team's deaths (a death to no
//!   player counts too: 85 = blue's deaths, red's kills were 84).
//! - `00 61` (`parse_clock`): the match's phase (1 preparing, 2 fought,
//!   3 over) with its start and end (server ms) — phase 2 ran 600 000 ms.
//! - `28 e3` (`parse_result`): your result as the match ends — `03` won,
//!   `02` lost (also in the 1v1 and 5v5 captures of 2026-11-12).
//! - `25 e3`: the kill feed, killer and victim by roster key (not read).

use crate::combat::nearby_players::NearbyPlayer;
use crate::entity::job_class::JobClass;

/// The arena maps (`ArenaCombat`): 1v1 and 5v5 alike. 61 seen in a capture.
pub const SOLO_ARENA_MAPS: [i32; 2] = [60, 61];

/// The battlegrounds (`ArenaCar`, `ArenaCTF`, `ArenaDom`): 10v10. 62 seen
/// in a capture (2026-11-13, "Arena of Tactics").
pub const BATTLEGROUND_MAPS: [i32; 3] = [62, 63, 64];

fn is_arena_map(map: i32) -> bool {
    SOLO_ARENA_MAPS.contains(&map) || BATTLEGROUND_MAPS.contains(&map)
}

/// One player on an arena roster (`19 e3`), team 1 or 2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterEntry {
    /// The player's 8 bytes before their name, `<id u32> 00 00 <home server
    /// u16>`, read as one number: how the match's kill and score records
    /// (`25 e3`, `26 e3`) name them.
    pub key: u64,
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
        let key = u64::from_le_bytes(body[i..i + 8].try_into().unwrap());
        out.push(RosterEntry { key, name, team: team.max(1), class, server });
        i = after + 13;
    }
    let teams: std::collections::HashSet<u8> = out.iter().map(|e| e.team).collect();
    (out.len() >= 2 && teams.len() == 2).then_some(out)
}

/// The kills, deaths and assists of the players a kill changed (`26 e3`),
/// by roster key: `<count u8>`, then each player `<key, 8 bytes>
/// <kills u8> <deaths u8> <assists u8> <u8>` — the first of every four with
/// one more byte (a flag) right after its key. None for a body of another
/// length. Seen 2026-11-13: 1 to 9 players and all 20, every one adding up.
pub fn parse_stats(body: &[u8]) -> Option<Vec<(u64, [u8; 3])>> {
    let n = *body.first()? as usize;
    if n == 0 || body.len() != 1 + n * 12 + n.div_ceil(4) {
        return None;
    }
    let mut out = Vec::with_capacity(n);
    let mut at = 1;
    for i in 0..n {
        let key = u64::from_le_bytes(body[at..at + 8].try_into().unwrap());
        at += if i % 4 == 0 { 9 } else { 8 };
        out.push((key, [body[at], body[at + 1], body[at + 2]]));
        at += 4;
    }
    Some(out)
}

/// The match's clock (`00 61`): `<u32> <phase u8> <start u64 ms> <end u64
/// ms>` — phase 1 the preparation, 2 the match, 3 over. Seen 2026-11-13:
/// phase 2 from 11:22:03 to 11:32:03 (600 000 ms), 3 at 11:29:27 when red
/// had won.
pub fn parse_clock(body: &[u8]) -> Option<(u8, i64, i64)> {
    if body.len() != 21 || !(1..=3).contains(&body[4]) {
        return None;
    }
    let ms = |at: usize| i64::from_le_bytes(body[at..at + 8].try_into().unwrap());
    let (start, end) = (ms(5), ms(13));
    (start > 0 && end >= start).then_some((body[4], start, end))
}

/// Your result as a match ends (`28 e3`): `<u8> <u32> <result u8> 01 <your
/// key> ...` — `03` won, `02` lost. True for won. Seen in all four
/// captures with an end (1v1 3:0 and 2:3, 5v5 lost, 10v10 won).
pub fn parse_result(body: &[u8]) -> Option<bool> {
    if body.len() < 15 || body[6] != 1 {
        return None;
    }
    match body[5] {
        3 => Some(true),
        2 => Some(false),
        _ => None,
    }
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

/// A player of a team match (from the roster), and what the match did to them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamPlayer {
    /// Their roster key (`RosterEntry::key`).
    pub key: u64,
    pub name: String,
    pub team: u8,
    pub class: Option<JobClass>,
    pub server: Option<u16>,
    pub item_level: Option<u32>,
    /// Their entity — UNKNOWN_ID until they spawned (or, you, hit).
    pub id: i32,
    /// Their damage on the other team, and the other team's on them.
    pub dealt: u64,
    pub taken: u64,
    /// The game's count once a kill record (`26 e3`) named them; until then
    /// deaths are their 0 HPs seen.
    pub kills: u32,
    pub deaths: u32,
    pub assists: u32,
    /// At 0 HP now.
    pub dead: bool,
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
    /// A team match's players, the roster's order — empty in a 1v1.
    pub players: Vec<TeamPlayer>,
    /// Your team in a team match, once your name is known.
    pub my_team: Option<u8>,
    /// A battleground (10v10): no rounds, the game's score and clock.
    pub battleground: bool,
    /// When the match's clock runs out (server ms, `00 61`).
    pub ends_ms: Option<i64>,
    /// The game said who won (`28 e3`).
    pub result: Option<Side>,
    /// The game's kill records (`26 e3`) came: they count kills, deaths and
    /// assists, not the 0 HPs seen.
    pub stats_seen: bool,
}

impl ArenaMatch {
    /// More than one a side: a team match.
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
        if is_arena_map(map) {
            if self.open().is_some_and(|m| m.map == map) {
                return;
            }
            self.close(now_ms);
            self.hp.clear();
            self.matches.push(ArenaMatch {
                map,
                entered_ms: now_ms,
                left_ms: None,
                opponent: None,
                rounds: Vec::new(),
                roster: None,
                players: Vec::new(),
                my_team: None,
                battleground: BATTLEGROUND_MAPS.contains(&map),
                ends_ms: None,
                result: None,
                stats_seen: false,
            });
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
        if m.is_team() {
            let roster = m.roster.clone().unwrap_or_default();
            if m.players.is_empty() {
                m.players = roster
                    .iter()
                    .map(|e| TeamPlayer {
                        key: e.key,
                        name: e.name.clone(),
                        team: e.team,
                        class: e.class,
                        server: e.server,
                        item_level: None,
                        id: UNKNOWN_ID,
                        dealt: 0,
                        taken: 0,
                        kills: 0,
                        deaths: 0,
                        assists: 0,
                        dead: false,
                    })
                    .collect();
            }
            m.my_team = roster.iter().find(|e| e.name == my_name).map(|e| e.team);
            let me = self.me;
            if let Some(t) = me.and_then(|me| self.open()?.players.iter_mut().find(|t| t.name == my_name).map(|t| (t, me))) {
                t.0.id = t.1;
            }
            return;
        }
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

    /// The players the roster named whose entity isn't known yet: the 1v1
    /// opponent, or a team match's players.
    pub fn players_without_entity(&self) -> Vec<String> {
        let Some(m) = self.current() else { return Vec::new() };
        let mut out: Vec<String> = m.opponent.iter().filter(|o| o.id == UNKNOWN_ID).map(|o| o.name.clone()).collect();
        out.extend(m.players.iter().filter(|p| p.id == UNKNOWN_ID).map(|p| p.name.clone()));
        out
    }

    /// A player spawned in sight: the opponent the roster named gets their
    /// entity, and their item level.
    pub fn player_spawned(&mut self, p: &NearbyPlayer) {
        let Some(m) = self.open() else { return };
        if let Some(t) = m.players.iter_mut().find(|t| t.name == p.name) {
            t.id = p.id;
            t.class = t.class.or(p.class);
            t.item_level = p.item_level.or(t.item_level);
            t.server = t.server.or(p.server);
        }
        let Some(o) = m.opponent.as_mut() else { return };
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
        if self.current().is_some_and(ArenaMatch::is_team) {
            self.team_hit(me, actor, target, damage, now_ms);
            return;
        }
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

    /// Your entity in a team match: you never spawn in your own sight.
    pub fn name_me(&mut self, me: i32, my_name: Option<&str>) {
        self.me = Some(me);
        let Some(name) = my_name else { return };
        if let Some(t) = self.open().and_then(|m| m.players.iter_mut().find(|t| t.name == name)) {
            t.id = me;
        }
    }

    /// A hit in a team match: between two players of different teams it is
    /// damage dealt and taken, and starts a round while both teams stand.
    fn team_hit(&mut self, me: i32, actor: i32, target: i32, damage: u64, now_ms: i64) {
        if damage == 0 || actor == target {
            return;
        }
        let Some(m) = self.open() else { return };
        let (Some(a), Some(t)) = (m.players.iter().position(|p| p.id == actor), m.players.iter().position(|p| p.id == target)) else { return };
        if m.players[a].team == m.players[t].team {
            return;
        }
        if m.battleground {
            // No rounds: what each player did is all of it.
            m.players[a].dealt += damage;
            m.players[t].taken += damage;
            return;
        }
        let standing = |team: u8, players: &[TeamPlayer]| players.iter().any(|p| p.team == team && !p.dead);
        if m.rounds.last().is_none_or(|r| r.ended_ms.is_some()) {
            if !(standing(1, &m.players) && standing(2, &m.players)) {
                return;
            }
            m.rounds.push(Round { started_ms: now_ms, ended_ms: None, winner: None, dealt: 0, taken: 0 });
        }
        m.players[a].dealt += damage;
        m.players[t].taken += damage;
        let round = m.rounds.last_mut().expect("a round");
        if actor == me {
            round.dealt += damage;
        } else if target == me {
            round.taken += damage;
        }
    }

    /// A team match's player at `hp`: 0 is a death, and the round ends when
    /// a whole team is down — won by the other.
    fn team_hp(&mut self, entity: i32, hp: i64, now_ms: i64) {
        let Some(m) = self.open() else { return };
        let Some(p) = m.players.iter_mut().find(|p| p.id == entity) else { return };
        if hp > 0 {
            p.dead = false;
            return;
        }
        if p.dead {
            return;
        }
        p.dead = true;
        if !m.stats_seen {
            p.deaths += 1;
        }
        let team = p.team;
        // A battleground has no rounds: everyone comes back.
        if m.battleground || m.players.iter().any(|p| p.team == team && !p.dead) {
            return;
        }
        let my_team = m.my_team;
        if let Some(round) = m.rounds.last_mut().filter(|r| r.ended_ms.is_none()) {
            round.ended_ms = Some(now_ms);
            round.winner = my_team.map(|mine| if mine == team { Side::Them } else { Side::Me });
        }
    }

    /// The HP of a player, as the game sent it: 0 for you or the opponent
    /// ends the round, won by the other.
    pub fn hp(&mut self, entity: i32, hp: i64, now_ms: i64) {
        if self.current().is_some_and(ArenaMatch::is_team) {
            self.team_hp(entity, hp, now_ms);
            return;
        }
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

    /// The game's kills, deaths and assists of the players a kill changed
    /// (`parse_stats`), for the team match being fought.
    pub fn stats(&mut self, stats: &[(u64, [u8; 3])]) {
        let Some(m) = self.open() else { return };
        if m.players.is_empty() {
            return;
        }
        m.stats_seen = true;
        for (key, [k, d, a]) in stats {
            if let Some(p) = m.players.iter_mut().find(|p| p.key == *key) {
                (p.kills, p.deaths, p.assists) = (u32::from(*k), u32::from(*d), u32::from(*a));
            }
        }
    }

    /// The match's clock (`parse_clock`): the end of the fought phase.
    pub fn clock(&mut self, phase: u8, end_ms: i64) {
        if let Some(m) = self.open().filter(|m| m.battleground && phase == 2) {
            m.ends_ms = Some(end_ms);
        }
    }

    /// The game's result as the match ends (`parse_result`).
    pub fn result(&mut self, won: bool) {
        if let Some(m) = self.open() {
            m.result = Some(if won { Side::Me } else { Side::Them });
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
                RosterEntry { key: 0x0909_0000_0002_6F3B, name: "Bradcrit".into(), team: 1, class: Some(JobClass::Gladiator), server: Some(2313) },
                RosterEntry { key: 0x0517_0000_0003_FD97, name: "StayHype".into(), team: 2, class: Some(JobClass::Gladiator), server: Some(1303) },
            ]
        );
    }

    #[test]
    fn a_team_match_counts_every_player_and_ends_with_a_wipe() {
        let mut a = ArenaTracker::default();
        a.map_loaded(60, 0);
        // P0 - P4 team 1, P5 - P9 team 2; you are P5 (entity ME).
        let roster = (0..10).map(|i| RosterEntry { key: i as u64, name: format!("P{i}"), team: if i < 5 { 1 } else { 2 }, class: None, server: None }).collect();
        a.roster(roster, Some("P5"));
        a.name_me(ME, Some("P5"));
        for i in (0..10).filter(|&i| i != 5) {
            let mut p = opp();
            p.id = 100 + i;
            p.name = format!("P{i}");
            a.player_spawned(&p);
        }
        assert!(a.players_without_entity().is_empty());
        a.hit(ME, 100, ME, 500, None, 1_000); // P0 on you
        a.hit(ME, ME, 101, 300, None, 1_100); // you on P1
        a.hit(ME, 106, 107, 999, None, 1_200); // P6 on P7: one team, left out
        a.hit(ME, 104, 108, 700, None, 1_300); // P4 on P8
        let m = a.current().unwrap();
        assert_eq!(m.my_team, Some(2));
        assert_eq!(m.rounds.len(), 1);
        assert_eq!((m.rounds[0].dealt, m.rounds[0].taken), (300, 500));
        assert_eq!((m.players[0].dealt, m.players[5].taken, m.players[8].taken, m.players[7].taken), (500, 500, 700, 0));
        for id in [ME, 106, 107, 108] {
            a.hp(id, 0, 2_000);
        }
        assert!(a.current().unwrap().rounds[0].ended_ms.is_none(), "one of your team stands");
        a.hp(109, 0, 3_000);
        let m = a.current().unwrap();
        assert_eq!(m.rounds[0].winner, Some(Side::Them));
        assert_eq!(m.score(), (0, 1));
        assert_eq!(m.players.iter().map(|p| p.deaths).sum::<u32>(), 5);
    }

    #[test]
    fn the_roster_names_the_opponent_before_the_first_hit() {
        let mut a = ArenaTracker::default();
        a.map_loaded(60, 0);
        let entry = |name: &str, team, class| RosterEntry { key: 0, name: name.into(), team, class, server: Some(2304) };
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

    fn bytes(hex: &str) -> Vec<u8> {
        let hex: String = hex.split_whitespace().collect();
        (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn the_battleground_s_records_read_as_the_game_showed_them() {
        // 11:22:29 of the capture of 2026-11-13, the first kill: the killer,
        // the victim and three with an assist — the first and the fifth with
        // the extra byte.
        let stats = parse_stats(&bytes(
            "05 F7BA000000002805 00 01000000 DE88000000002905 00010000 0D81020000000209 00000100 \
             55DF020000000909 00000100 C31C010000000F09 00 00000100",
        ))
        .expect("five players");
        assert_eq!(stats.len(), 5);
        assert_eq!(stats[0], (0x0528_0000_0000_BAF7, [1, 0, 0]));
        assert_eq!(stats[1].1, [0, 1, 0]);
        assert_eq!(stats[4], (0x090F_0000_0001_1CC3, [0, 0, 1]));
        assert!(parse_stats(&bytes("05 F7BA000000002805 00 01000000")).is_none(), "cut short");

        let clock = parse_clock(&bytes("0A000000 02 3FBD6DD4A1010000 FFE476D4A1010000")).expect("the clock");
        assert_eq!((clock.0, clock.2 - clock.1), (2, 600_000));

        // Won (the 10v10), lost (the 5v5 of 2026-11-12).
        assert_eq!(parse_result(&bytes("02 C4050000 03 01 3B6F020000000909 04081600")), Some(true));
        assert_eq!(parse_result(&bytes("01 DD050000 02 01 3B6F020000000909 00010000")), Some(false));
    }

    #[test]
    fn a_battleground_counts_the_game_s_score_and_has_no_rounds() {
        let mut a = ArenaTracker::default();
        a.map_loaded(62, 0);
        let roster = (0..20).map(|i| RosterEntry { key: 1000 + i as u64, name: format!("P{i}"), team: if i < 10 { 1 } else { 2 }, class: None, server: None }).collect();
        a.roster(roster, Some("P10"));
        a.name_me(ME, Some("P10"));
        let mut p = opp();
        p.id = 100;
        p.name = "P0".into();
        a.player_spawned(&p);
        a.clock(2, 600_000);
        a.hit(ME, ME, 100, 700, None, 1_000);
        a.hp(100, 0, 1_100);
        a.stats(&[(1010, [1, 0, 0]), (1000, [0, 1, 0])]);
        // You die and respawn: the map loads again, the match goes on.
        a.map_loaded(62, 2_000);
        a.hp(100, 0, 3_000);
        a.result(true);
        let m = a.current().expect("still open");
        assert!(m.battleground && m.rounds.is_empty());
        assert_eq!(m.ends_ms, Some(600_000));
        assert_eq!(m.result, Some(Side::Me));
        assert_eq!((m.players[10].dealt, m.players[0].taken), (700, 700));
        assert_eq!((m.players[10].kills, m.players[0].deaths), (1, 1), "the game's count, not each 0 HP");
    }
}
