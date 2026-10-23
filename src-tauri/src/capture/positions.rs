//! Where entities are in the world, as the game tells it — for the pet
//! tracker's distances (gh_engine `npcs`) and your own spot (`self.pos`).
//!
//! A position is three `f32` LE (x, y, z) in the game's units (cm: running
//! covers ~5 m/s). The game never sends your own movement back to you — the
//! client sends it, encrypted — so your spot comes only from records that
//! happen to carry it:
//!
//! - `23 36 00 <5 × 00> <pos>`: you were put somewhere (a zone load, a
//!   teleport). Entity 0 is you.
//! - `2A 38 <entity> 01 <flags u8> <instance varint> <effect u32> <duration
//!   u64> <time u64> <source varint> <varint> [flags 0x13: <u32>] <u8> <pos>`:
//!   an effect lands on an entity, at its spot. Every buff, debuff and hit
//!   effect on you — several a second in a fight, every few seconds while
//!   running. (Its sibling `2B 38` repeats the spot an effect was first
//!   applied at: old, not used.)
//! - `02 38 <caster> 00 <skill u32> <u8> <flags u8> <target varint> <f32>
//!   [flags & 2: <pos>]`: a skill used at a target, at the target's spot —
//!   yours when you aim it at yourself, the mob's otherwise.
//! - `1A/1B/1C/3C 37 <entity> <u8> [<u8>] <pos>`: a mob or player moves (one
//!   or two bytes before the spot, by the first one's kind — both are tried).
//! - `41 36 <entity> <u8> <u16> <u32> 00 02 <pos>`: a mob or summon spawns.
//!
//! Read off two captures (2026-10-23, 22:20 and 22:32): teleport, run, three
//! kills, run on. The spots of the effects on you followed the run at ~5 m/s
//! and met the mobs' own spots (moves, spawns) within skill range at each kill.

use super::stream_processor::read_varint;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Position {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Position {
    /// The distance on the ground (x/y) in metres.
    pub fn metres_to(&self, other: &Position) -> f32 {
        ((self.x - other.x).powi(2) + (self.y - other.y).powi(2)).sqrt() / 100.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Spotted {
    /// You, put somewhere (`23 36` with entity 0).
    You(Position),
    /// An entity (you among them) at a spot.
    Entity(i32, Position),
}

/// The spot one game packet (a whole frame) tells of, if any.
pub fn parse(packet: &[u8]) -> Option<Spotted> {
    let length = read_varint(packet, 0);
    if length.length <= 0 {
        return None;
    }
    let at = length.length as usize;
    let op = packet.get(at..at + 2)?;
    let entity = read_varint(packet, at + 2);
    if entity.length <= 0 {
        return None;
    }
    let after = at + 2 + entity.length as usize;
    match op {
        [0x23, 0x36] if entity.value == 0 => position_at(packet, after + 5).map(Spotted::You),
        [0x2A, 0x38] => effect(packet, after).map(|pos| Spotted::Entity(entity.value, pos)),
        [0x02, 0x38] => skill_target(packet, after),
        [0x1A | 0x1B | 0x1C | 0x3C, 0x37] => {
            position_at(packet, after + 1).or_else(|| position_at(packet, after + 2)).map(|pos| Spotted::Entity(entity.value, pos))
        }
        [0x41, 0x36] if packet.get(after + 8) == Some(&0x02) => position_at(packet, after + 9).map(|pos| Spotted::Entity(entity.value, pos)),
        _ => None,
    }
}

fn effect(packet: &[u8], mut i: usize) -> Option<Position> {
    if packet.get(i) != Some(&0x01) {
        return None;
    }
    let flags = *packet.get(i + 1)?;
    i += 2;
    i += varint_len(packet, i)?;
    i += 4 + 8 + 8; // effect, duration, time
    i += varint_len(packet, i)?; // source
    i += varint_len(packet, i)?;
    if flags == 0x13 {
        i += 4;
    }
    position_at(packet, i + 1)
}

fn skill_target(packet: &[u8], i: usize) -> Option<Spotted> {
    let flags = *packet.get(i + 6)?;
    if flags & 0x02 == 0 {
        return None;
    }
    let target = read_varint(packet, i + 7);
    if target.length <= 0 {
        return None;
    }
    position_at(packet, i + 7 + target.length as usize + 4).map(|pos| Spotted::Entity(target.value, pos))
}

fn varint_len(packet: &[u8], i: usize) -> Option<usize> {
    let v = read_varint(packet, i);
    (v.length > 0).then_some(v.length as usize)
}

/// Three floats that can be a spot in the world: the maps span a few
/// kilometres either way, never close to 0 on both axes — anything else is
/// the wrong offset.
pub(crate) fn position_at(packet: &[u8], i: usize) -> Option<Position> {
    let f = |k: usize| packet.get(i + 4 * k..i + 4 * k + 4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let pos = Position { x: f(0)?, y: f(1)?, z: f(2)? };
    let plausible = |v: f32| v.is_finite() && v.abs() < 5_000_000.0;
    (plausible(pos.x) && plausible(pos.y) && plausible(pos.z) && pos.x.abs().max(pos.y.abs()) > 1_000.0).then_some(pos)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    fn rounded(p: Position) -> (i32, i32, i32) {
        (p.x.round() as i32, p.y.round() as i32, p.z.round() as i32)
    }

    fn entity(packet: &str) -> (i32, (i32, i32, i32)) {
        match parse(&hex(packet)) {
            Some(Spotted::Entity(id, pos)) => (id, rounded(pos)),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_teleport_puts_you_somewhere() {
        let Some(Spotted::You(pos)) = parse(&hex("1e2336000000000000fd523848f344b4c70078724605000000c566")) else { panic!() };
        assert_eq!(rounded(pos), (188748, -92298, 15518));
    }

    #[test]
    fn an_effect_on_you_says_where_you_are() {
        // flags 0x13 (with the u32) and 0x11 (without).
        assert_eq!(
            entity("342a38fb6d0113c101c7890807b80b000000000000eebe6b6aa1010000fb6d12c70db400000b1922486f0faac700805346"),
            (14075, (165988, -87071, 13536))
        );
        assert_eq!(
            entity("302a38fb6d0111890202e1f505ffffffffffffffff8075d52abb030000fb6d01001e103848d89bb4c7c1267346"),
            (14075, (188480, -92472, 15562))
        );
    }

    #[test]
    fn a_skill_at_a_target_says_where_the_target_is() {
        assert_eq!(entity("260238fb6d006c27a800b102f78a03d64bd241b342224848e6a9c700885346df5f0100"), (50551, (166155, -86989, 13538)));
        // No spot (flags 0): nothing.
        assert_eq!(parse(&hex("190238fb6d000f27a800b300fb6dd64bd241df5f0100")), None);
    }

    #[test]
    fn moves_and_spawns_say_where_mobs_are() {
        // One byte before the spot, then two.
        assert_eq!(entity("1b1c37cfad0104a3bf1f482753a7c700044c4676a676a601").0, 22223);
        assert_eq!(entity("1b1c37cfad010503a3bf1f482753a7c700044c4676a676a6").0, 22223);
        let (_, pos) = entity("1b1c37cfad010503a3bf1f482753a7c700044c4676a676a6");
        assert_eq!(pos, (163583, -85670, 13057));
        let (_, pos) = entity("4c4136fe8603052000aba224000002fb5330488a9fb1c700d47f46984a60437f9f0194a70794a70764");
        assert_eq!(pos, (180560, -90943, 16373));
    }
}
