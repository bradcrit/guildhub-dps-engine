//! The world's objects in range — gathering spots, chests, Hidden Cubes — as
//! the game tells them, for the Hidden Cube hint (gh_engine `cubes`).
//!
//! - `34 36 <entity> <u16 flags> <code u32> <pos>`: an object comes into
//!   range (or is there at a zone load), with its spot (x, y, z f32 LE).
//! - `35 36 <entity> <u8> <u8> 00`: it is gone — `01 01` out of range,
//!   `04 03` used up (opened, gathered — by you or anyone).
//!
//! Read off two captures (2026-10-24, 00:28 and 00:36): the objects came and
//! went with the run, and the two Hidden Cubes opened (code HIDDEN_CUBE, in
//! two zones) went with `04 03` right after the game's open animation. The
//! game sends an object some 100 m or more away — further than its sound.

use super::positions::{position_at, Position};
use super::stream_processor::read_varint;

/// The object code of a Hidden Cube.
pub const HIDDEN_CUBE: u32 = 1_301_199;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ObjectRecord {
    /// An object in range: its entity, code and spot.
    InRange(i32, u32, Position),
    /// It is gone (out of range or used up).
    Gone(i32),
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
        [0x35, 0x36] if packet.get(after + 2) == Some(&0x00) => Some(ObjectRecord::Gone(entity.value)),
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
            (56408, HIDDEN_CUBE, (165219, 149794, 19855))
        );
        assert_eq!(
            in_range("3D3436A8B1020400CFDA130080F4EFC7004090C500FC0646000000000050A0430101012B80F16AA10100000000000000000000077F0B69070000"),
            (39080, HIDDEN_CUBE, (-122857, -4616, 8639))
        );
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
        assert_eq!(parse(&hex("0C3536FC8102010100")), Some(ObjectRecord::Gone(33020)));
        assert_eq!(parse(&hex("0C3536D8B803040300")), Some(ObjectRecord::Gone(56408)));
        assert_eq!(parse(&hex("0B4236D0F0010001")), None);
    }
}
