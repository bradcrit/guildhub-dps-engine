//! The world's objects in range — gathering spots, chests, Hidden Cubes — as
//! the game tells them, for the Hidden Cube hint (gh_engine `cubes`).
//!
//! - `34 36 <entity> <u16 flags> <code u32> <pos>`: an object comes into
//!   range (or is there at a zone load), with its spot (x, y, z f32 LE).
//!   Past the spot: `<f32> <heading f32> 01 01 01 <created u64>`, when the
//!   object came to be (unix ms) — a fresh cube's a second before it came,
//!   a static object's the server's start.
//! - `35 36 <entity> <u8> <u8> 00`: it is gone — `01 01` out of range,
//!   `04 03` used up (opened, gathered — by you or anyone).
//! - `38 36 <entity> <actor varint> 01`: someone starts to open (gather) it.
//! - `3A 36 <entity> <actor varint> <count u32> <item u32>`: they are done,
//!   with the item it took — a Hidden Cube Key (HIDDEN_CUBE_KEY) for a cube,
//!   0 for most else (gathering, 2026-10-25).
//!
//! Read off two captures (2026-10-24, 00:28 and 00:36): the objects came and
//! went with the run, and the two Hidden Cubes opened (code 1301199, in two
//! zones) went with `04 03` right after the game's open animation. The game
//! sends an object some 100 m or more away — further than its sound.
//!
//! Cubes belong to a faction's own lands, each side's with its own codes (see
//! HIDDEN_CUBES). A guess by the codes' pattern (1201199 for Verteron) was
//! wrong: an Elyos log of 2026-10-25 had 1200039 in Verteron instead.
//!
//! The key, the open's records and the created time as dr89mh/aion2-cube-radar
//! (docs/FINDINGS.md B6, B10, O1) found them; the open's layout checked against
//! our own gathering (2026-10-25).
//!
//! To find a code from a log, every other object's code is logged once a map
//! when it comes into range, and again when one close to you is used up —
//! right before the reward list of one you opened (`DataStorage::note_object`).

use super::positions::{position_at, Position};
use super::stream_processor::read_varint;

/// The object codes of the Hidden Cubes (the game's current "V04" ones).
/// Elyos lands (Verteron): 1200039–1200041, `E_HiddenCube_01_V04_001`–`003` —
/// 1200039 and 1200041 opened with a key in others' captures (2026-10-13),
/// 1200039 in an Elyos log of ours. Asmodian lands (Altgard): 1301199 (our
/// captures) and 1301200 (`E_D1_HiddenCube_02`, by the game data's names).
pub const HIDDEN_CUBES: &[u32] = &[1_200_039, 1_200_040, 1_200_041, 1_301_199, 1_301_200];

/// The item a Hidden Cube's open takes: its key.
pub const HIDDEN_CUBE_KEY: u32 = 630_900_003;

/// Whether an object code is a Hidden Cube's.
pub fn is_hidden_cube(code: u32) -> bool {
    HIDDEN_CUBES.contains(&code)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ObjectRecord {
    /// An object in range: its entity, code, spot and when it came to be
    /// (unix ms, None where the record has none).
    InRange(i32, u32, Position, Option<i64>),
    /// Someone (`actor`) starts to open it: entity, actor.
    Opening(i32, i32),
    /// They are done: entity, actor, and the item it took (0: none).
    Opened(i32, i32, u32),
    /// It is gone: its entity, and whether it was used up (`04 03`: opened,
    /// gathered) rather than out of range.
    Gone(i32, bool),
}

/// The object record one game packet (a whole frame) is, if any.
pub fn parse(packet: &[u8]) -> Option<ObjectRecord> {
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
        [0x34, 0x36] => {
            let code = u32_at(packet, after + 2)?;
            let pos = position_at(packet, after + 6)?;
            // Generous: only to turn away bytes that aren't a time.
            let created = (packet.get(after + 26..after + 29) == Some(&[1, 1, 1][..]))
                .then(|| packet.get(after + 29..after + 37))
                .flatten()
                .map(|b| i64::from_le_bytes(b.try_into().unwrap()))
                .filter(|ms| (1_600_000_000_000..4_000_000_000_000).contains(ms));
            Some(ObjectRecord::InRange(entity.value, code, pos, created))
        }
        [0x38, 0x36] => {
            let actor = read_varint(packet, after);
            (actor.length > 0).then_some(ObjectRecord::Opening(entity.value, actor.value))
        }
        [0x3A, 0x36] => {
            let actor = read_varint(packet, after);
            if actor.length <= 0 {
                return None;
            }
            let item = u32_at(packet, after + actor.length as usize + 4)?;
            Some(ObjectRecord::Opened(entity.value, actor.value, item))
        }
        [0x35, 0x36] if packet.get(after + 2) == Some(&0x00) => {
            Some(ObjectRecord::Gone(entity.value, packet.get(after..after + 2) == Some(&[0x04, 0x03][..])))
        }
        _ => None,
    }
}

fn u32_at(packet: &[u8], at: usize) -> Option<u32> {
    packet.get(at..at + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    fn in_range(packet: &str) -> (i32, u32, (i32, i32, i32)) {
        match parse(&hex(packet)) {
            Some(ObjectRecord::InRange(id, code, p, _)) => (id, code, (p.x.round() as i32, p.y.round() as i32, p.z.round() as i32)),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn hidden_cubes_come_into_range() {
        assert_eq!(
            in_range("3D3436D8B8030400CFDA1300CE58214878481248001E9B4600000000003084430101016891E06AA10100000000000000000000369FA8F0070000"),
            (56408, 1_301_199, (165219, 149794, 19855))
        );
        assert_eq!(
            in_range("3D3436A8B1020400CFDA130080F4EFC7004090C500FC0646000000000050A0430101012B80F16AA10100000000000000000000077F0B69070000"),
            (39080, 1_301_199, (-122857, -4616, 8639))
        );
    }

    #[test]
    fn when_an_object_came_to_be() {
        let created = |packet: &str| match parse(&hex(packet)) {
            Some(ObjectRecord::InRange(.., created)) => created,
            other => panic!("{other:?}"),
        };
        assert_eq!(
            created("3D3436D8B8030400CFDA1300CE58214878481248001E9B4600000000003084430101016891E06AA10100000000000000000000369FA8F0070000"),
            Some(1_792_794_464_616)
        );
        // Not `01 01 01` before it: no time.
        assert_eq!(created("3D3436D8B8030400CFDA1300CE58214878481248001E9B4600000000003084430101026891E06AA101000000000000"), None);
    }

    #[test]
    fn opened_by_someone() {
        // Gathering of our own (2026-10-25): no item taken.
        assert_eq!(parse(&hex("0C3836F8A702CA7901")), Some(ObjectRecord::Opening(37880, 15562)));
        assert_eq!(parse(&hex("133A36F8A702CA79020000000000000000")), Some(ObjectRecord::Opened(37880, 15562, 0)));
        // A cube's (dr89mh's B6): one key.
        assert_eq!(parse(&hex("0F3A36E5FF03DC470100000023C59A25")), Some(ObjectRecord::Opened(65509, 9180, HIDDEN_CUBE_KEY)));
    }

    #[test]
    fn both_sides_cubes() {
        assert!(is_hidden_cube(1_301_199) && is_hidden_cube(1_200_039) && is_hidden_cube(1_200_041));
        // Other objects of either side, and the wrong guess.
        assert!(!is_hidden_cube(1_300_865) && !is_hidden_cube(1_200_865) && !is_hidden_cube(1_201_199));
    }

    #[test]
    fn other_objects_too() {
        // Flags 0x0C: more after the spot, the same before it.
        let (id, code, _) = in_range("413436F894020C0081D91300C304014824271F48003681460000000000000000010101ECF43D5EA10100000000000000000000387C3FDB09000582730700");
        assert_eq!((id, code), (35448, 1_300_865));
    }

    #[test]
    fn objects_go() {
        // Out of range, and used up.
        assert_eq!(parse(&hex("0C3536FC8102010100")), Some(ObjectRecord::Gone(33020, false)));
        assert_eq!(parse(&hex("0C3536D8B803040300")), Some(ObjectRecord::Gone(56408, true)));
        assert_eq!(parse(&hex("0B4236D0F0010001")), None);
    }
}
