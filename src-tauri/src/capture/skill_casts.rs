//! Skills used, your running cooldowns and who is in combat — for
//! GuildHub Companion's cooldown bar and a boss's cast bar.
//!
//! ```text
//! <len> 02 38 <actor> <flags u8> <skill u32> <u8> [flags & 1: <u8>]
//!             <target flags u8> <target> [target flags & 1: <u32>] <yaw f32>
//!             [target flags & 2: <x f32> <y f32> <z f32>]
//!             <speed varint> <u8> [flags & 2: <varint>] <cooldown ms varint>   a skill used
//! <len> 47 38 <n u8> n × (<skill u32> <ms left varint>)                         your cooldowns moved
//! <len> 21 8d <entity> <varint> <in combat varint 0/1> [<deadline u64>]          in or out of combat
//! ```
//!
//! - `02 38` comes for anyone in sight as the skill starts: players, their
//!   summons and monsters. `cooldown` is the server's, every reduction
//!   included; 0 for a skill without one (or a chain step). `speed` is the
//!   caster's attack speed in hundredths of a percent (10000 = 100 %).
//!   A monster's skill lands some time after its `02 38` — its cast time —
//!   which the record doesn't carry; `DataStorage::note_skill_hit` learns it
//!   from the first hit that follows (2026-10-25: the same monster skill
//!   600 ± 50 ms, another 1100 ± 60 ms, every time).
//! - `47 38` is only ever about your own skills: the cooldowns the server
//!   moved (a reduction, a reset, a cooldown running when you log in).
//! - `21 8d` carries a `deadline` (unix ms) when a boss with a time limit is
//!   pulled (Ferocious Horn Nuakum: 5 minutes).
//!
//! Layouts as read by Taeloria's Spellbook (taylorjohannsen/taelorias-
//! spellbook-aion2-cooldown-tracker, MIT, `src/packets.rs`), whose test
//! records came from cyberbadger6969/aion2-dps-meter. Checked against our own
//! captures of 2026-10-25: all 1000 `02 38` (924 players', 63 monsters') and
//! 258 `21 8d` read to their last byte.

use super::stream_processor::read_varint;

const CAST: [u8; 2] = [0x02, 0x38];
const COOLDOWNS: [u8; 2] = [0x47, 0x38];
const BATTLE: [u8; 2] = [0x21, 0x8D];

/// Longer is not a cooldown: the record was misread.
const MAX_COOLDOWN_MS: i32 = 3_600_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cast {
    pub actor: i32,
    pub skill: u32,
    pub target: i32,
    pub cooldown_ms: u32,
    /// Attack speed, 10000 = 100 %.
    pub speed: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    Cast(Cast),
    /// Your own skills: (skill, ms left).
    Cooldowns(Vec<(u32, u32)>),
    Battle { entity: i32, on: bool, deadline_ms: Option<i64> },
}

/// A skill without its specialization digits (the last four) — the id the
/// skill bar, NC's site and the cooldowns go by.
pub fn base_skill(skill: u32) -> u32 {
    skill / 10_000 * 10_000
}

/// A monster's skill (seven digits), not a player's or an item's.
pub fn is_npc_skill(skill: u32) -> bool {
    (1_000_000..=9_999_999).contains(&skill)
}

struct Reader<'a> {
    b: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn varint(&mut self) -> Option<i32> {
        let v = read_varint(self.b, self.at);
        if v.length <= 0 {
            return None;
        }
        self.at += v.length as usize;
        Some(v.value)
    }
    fn u8(&mut self) -> Option<u8> {
        let v = *self.b.get(self.at)?;
        self.at += 1;
        Some(v)
    }
    fn u32(&mut self) -> Option<u32> {
        let v = u32::from_le_bytes(self.b.get(self.at..self.at + 4)?.try_into().ok()?);
        self.at += 4;
        Some(v)
    }
    fn skip(&mut self, n: usize) -> Option<()> {
        (self.at + n <= self.b.len()).then(|| self.at += n)
    }
    fn rest(&self) -> usize {
        self.b.len() - self.at
    }
}

fn is_entity(id: i32) -> bool {
    (1..=9_999_999).contains(&id)
}

fn is_skill(id: u32) -> bool {
    (1..300_000_000).contains(&id)
}

/// The record one game packet (a whole frame) is, if it is one of these.
pub fn parse(packet: &[u8]) -> Option<Record> {
    let length = read_varint(packet, 0);
    if length.length <= 0 {
        return None;
    }
    let at = length.length as usize;
    let body = packet.get(at + 2..)?;
    let mut r = Reader { b: body, at: 0 };
    match packet.get(at..at + 2)? {
        op if op == CAST => cast(&mut r).map(Record::Cast),
        op if op == COOLDOWNS => cooldowns(&mut r),
        op if op == BATTLE => battle(&mut r),
        _ => None,
    }
}

fn cast(r: &mut Reader) -> Option<Cast> {
    let actor = r.varint().filter(|&id| is_entity(id))?;
    let flags = r.u8()?;
    let skill = r.u32().filter(|&s| is_skill(s))?;
    r.u8()?;
    if flags & 0x01 != 0 {
        r.u8()?;
    }
    let target_flags = r.u8()?;
    let target = r.varint()?;
    if target_flags & 0x01 != 0 {
        r.u32()?;
    }
    r.skip(4)?;
    if target_flags & 0x02 != 0 {
        r.skip(12)?;
    }
    let speed = r.varint().filter(|&s| s >= 0)? as u32;
    r.u8()?;
    if flags & 0x02 != 0 {
        r.varint()?;
    }
    let cooldown_ms = r.varint().filter(|&ms| (0..=MAX_COOLDOWN_MS).contains(&ms))? as u32;
    Some(Cast { actor, skill, target, cooldown_ms, speed })
}

fn cooldowns(r: &mut Reader) -> Option<Record> {
    let count = r.u8().filter(|n| (1..=64).contains(n))?;
    let mut list = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let skill = r.u32().filter(|&s| is_skill(s))?;
        let left = r.varint().filter(|&ms| (0..=MAX_COOLDOWN_MS).contains(&ms))? as u32;
        list.push((skill, left));
    }
    Some(Record::Cooldowns(list))
}

fn battle(r: &mut Reader) -> Option<Record> {
    let entity = r.varint().filter(|&id| is_entity(id))?;
    r.varint()?;
    let on = match r.varint()? {
        0 => false,
        1 => true,
        _ => return None,
    };
    let deadline_ms = match r.rest() {
        0 => None,
        8 => Some(i64::from_le_bytes(r.b.get(r.at..r.at + 8)?.try_into().ok()?)),
        _ => return None,
    };
    Some(Record::Battle { entity, on, deadline_ms })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        let s: String = s.split_whitespace().collect();
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// A body as a framed packet (a one-byte length, which the parser skips).
    fn framed(opcode: [u8; 2], body: &[u8]) -> Vec<u8> {
        let mut p = vec![(body.len() + 3) as u8];
        p.extend(opcode);
        p.extend(body);
        p
    }

    // Our captures of 2026-10-25 (Verteron).
    #[test]
    fn a_monster_skill_and_a_summon() {
        let monster = hex("27023888d70200ded21200010296090c693042ab70e5476e7f794700c06e45904e01e05d");
        assert_eq!(
            parse(&monster),
            Some(Record::Cast(Cast { actor: 43912, skill: 1_233_630, target: 1174, cooldown_ms: 12_000, speed: 10_000 }))
        );
        let summon = hex("260238915d00b0d1f5001a02915dd488d942fb3ddb4771a5534712b00645904e01a471");
        assert_eq!(
            parse(&summon),
            Some(Record::Cast(Cast { actor: 11921, skill: 16_110_000, target: 11921, cooldown_ms: 14_500, speed: 10_000 }))
        );
        assert!(is_npc_skill(1_233_630) && !is_npc_skill(16_110_000));
        assert_eq!(base_skill(15_280_140), 15_280_000);
    }

    // Taeloria's Spellbook's tests (EU, 2026-10-05, from cyberbadger6969/aion2-dps-meter).
    #[test]
    fn a_sorcerer_on_a_boss() {
        let body = hex("c74300 0c28e900 9202 d1e802 b0f930c38dbae8470b71d3c700faa546ea5d01dc6f");
        let Some(Record::Cast(cast)) = parse(&framed(CAST, &body)) else { panic!() };
        assert_eq!((cast.actor, cast.target, cast.skill, cast.cooldown_ms), (8647, 46161, 15_280_140, 14_300));
        for len in 0..body.len() - 1 {
            assert_eq!(parse(&framed(CAST, &body[..len])), None, "length {len}");
        }
    }

    #[test]
    fn cooldown_list() {
        let mut body = vec![2];
        body.extend(18_010_000u32.to_le_bytes());
        body.extend([0xDC, 0x6F]);
        body.extend(18_040_000u32.to_le_bytes());
        body.push(0);
        assert_eq!(parse(&framed(COOLDOWNS, &body)), Some(Record::Cooldowns(vec![(18_010_000, 14_300), (18_040_000, 0)])));
        assert_eq!(parse(&framed(COOLDOWNS, &body[..body.len() - 1])), None);
    }

    #[test]
    fn combat_and_a_timed_boss() {
        assert_eq!(parse(&hex("0b218df6cd020001")), Some(Record::Battle { entity: 42742, on: true, deadline_ms: None }));
        assert_eq!(parse(&hex("0b218dbc8e020000")), Some(Record::Battle { entity: 34620, on: false, deadline_ms: None }));
        assert_eq!(
            parse(&framed(BATTLE, &hex("f19702 01 01 ea66962da1010000"))),
            Some(Record::Battle { entity: 35825, on: true, deadline_ms: Some(0x01a1_2d96_66ea) })
        );
        assert_eq!(parse(&framed(BATTLE, &hex("c8330002"))), None);
        assert_eq!(parse(&framed(BATTLE, &hex("c833000100"))), None);
    }
}
