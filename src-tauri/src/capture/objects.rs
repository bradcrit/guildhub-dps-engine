//! The world's objects in range — gathering spots, chests, Hidden Cubes — as
//! the game tells them, for the Hidden Cube hint (gh_engine `cubes`).
//!
//! - `34 36 <entity> <u16 flags> <code u32> <pos>`: an object comes into
//!   range (or is there at a zone load), with its spot (x, y, z f32 LE).
//! - `35 36 <entity> <u8> <u8> 00`: it is gone — `01 01` out of range,
//!   `04 03` used up (opened, gathered — by you or anyone).
//!
//! Read off two captures (2026-10-24, 00:28 and 00:36): the objects came and
//! went with the run, and the two Hidden Cubes opened (code
//! HIDDEN_CUBE_ASMODIAN, in two zones) went with `04 03` right after the
//! game's open animation. The game sends an object some 100 m or more away —
//! further than its sound.
//!
//! Cubes belong to a faction's own lands. The object codes go by side: in
//! Altgard (map 1110) 13…, in Verteron (map 1010) 12… with the same rest for
//! the same object (1300865/1200865, 1300824/1200824, 1300825/1200825 in a
//! log of 2026-10-25) — so the Elyos cube is taken to be 1201199, not seen
//! yet: an Asmodian in Verteron got none.
//!
//! To find a code from a log, every other object's code is logged once a map
//! when it comes into range, and again when one close to you is used up —
//! right before the reward list of one you opened (`DataStorage::note_object`).

use super::positions::{position_at, Position};
use super::stream_processor::read_varint;

/// The object code of a Hidden Cube in the Asmodian lands (Altgard).
pub const HIDDEN_CUBE_ASMODIAN: u32 = 1_301_199;
/// The object code of a Hidden Cube in the Elyos lands (Verteron) — by the
/// codes' pattern (see the top of this file), unconfirmed.
pub const HIDDEN_CUBE_ELYOS: u32 = 1_201_199;

/// Whether an object code is a Hidden Cube's.
pub fn is_hidden_cube(code: u32) -> bool {
    code == HIDDEN_CUBE_ASMODIAN || code == HIDDEN_CUBE_ELYOS
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ObjectRecord {
    /// An object in range: its entity, code and spot.
    InRange(i32, u32, Position),
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
            let code = packet.get(after + 2..after + 6)?;
            let code = u32::from_le_bytes([code[0], code[1], code[2], code[3]]);
            position_at(packet, after + 6).map(|pos| ObjectRecord::InRange(entity.value, code, pos))
        }
        [0x35, 0x36] if packet.get(after + 2) == Some(&0x00) => {
            Some(ObjectRecord::Gone(entity.value, packet.get(after..after + 2) == Some(&[0x04, 0x03][..])))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    fn in_range(packet: &str) -> (i32, u32, (i32, i32, i32)) {
        match parse(&hex(packet)) {
            Some(ObjectRecord::InRange(id, code, p)) => (id, code, (p.x.round() as i32, p.y.round() as i32, p.z.round() as i32)),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn hidden_cubes_come_into_range() {
        assert_eq!(
            in_range("3D3436D8B8030400CFDA1300CE58214878481248001E9B4600000000003084430101016891E06AA10100000000000000000000369FA8F0070000"),
            (56408, HIDDEN_CUBE_ASMODIAN, (165219, 149794, 19855))
        );
        assert_eq!(
            in_range("3D3436A8B1020400CFDA130080F4EFC7004090C500FC0646000000000050A0430101012B80F16AA10100000000000000000000077F0B69070000"),
            (39080, HIDDEN_CUBE_ASMODIAN, (-122857, -4616, 8639))
        );
    }

    #[test]
    fn both_sides_cubes() {
        assert!(is_hidden_cube(HIDDEN_CUBE_ASMODIAN) && is_hidden_cube(HIDDEN_CUBE_ELYOS));
        assert!(!is_hidden_cube(1_300_865) && !is_hidden_cube(1_200_865));
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
