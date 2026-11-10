//! What one round of your own gathering brought: `33 8D`. A node is gathered
//! in rounds (three in every one seen), each its own `38 36` … `3A 36`
//! (`capture::objects`, the round's number in place of the count), and each
//! round ends with this record — for you only, with no actor in it (another
//! player's gathering close by showed `3A 36` but no `33 8D`).
//!
//! ```text
//! 33 8D <outcome u8> 3A <node varint> <item u32> <count u32> <bonus u32> …
//! ```
//!
//! - outcome 0: the node's plain material, 1: its better one (one grade up,
//!   by id: 610660026 / 610660029 and 610610013 / 610610014 off two kinds of
//!   node), 2: failed — the item is then the plain material and the count 0.
//! - bonus: more of it on top — a round with count 1 and bonus 1 put 2 into
//!   the inventory (`1B 56`).
//! - A failed round brings nothing into the inventory (no `1B 56`).
//!
//! Read 2026-11-10 (23:05 capture): nine rounds on three nodes of two kinds,
//! the last one failed, as the player saw it in the game.

use super::stream_processor::read_varint;

const KIND: [u8; 2] = [0x33, 0x8D];
/// The byte after the outcome in every round seen.
const OBJECT_TAG: u8 = 0x3A;
/// Item ids are nine digits.
const ITEM_IDS: std::ops::RangeInclusive<u32> = 100_000_000..=999_999_999;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Plain,
    Better,
    Failed,
}

impl Outcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Plain => "plain",
            Outcome::Better => "better",
            Outcome::Failed => "failed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gathered {
    /// The node's entity (`capture::objects`).
    pub node: i32,
    pub outcome: Outcome,
    /// The game's item id — on a fail, the material it would have been.
    pub item: u32,
    /// What came into the inventory, the bonus included (0 on a fail).
    pub count: u32,
    /// Of `count`, what came on top.
    pub bonus: u32,
}

/// A whole frame (its length varint, type, body): the round it ends, if any.
pub fn read(frame: &[u8]) -> Option<Gathered> {
    let at = usize::try_from(read_varint(frame, 0).length).ok().filter(|&n| n > 0)?;
    if frame.get(at..at + 2)? != KIND {
        return None;
    }
    let outcome = match *frame.get(at + 2)? {
        0 => Outcome::Plain,
        1 => Outcome::Better,
        2 => Outcome::Failed,
        _ => return None,
    };
    if *frame.get(at + 3)? != OBJECT_TAG {
        return None;
    }
    let node = read_varint(frame, at + 4);
    if node.length <= 0 {
        return None;
    }
    let after = at + 4 + node.length as usize;
    let u32_at = |i: usize| frame.get(after + i..after + i + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()));
    let item = u32_at(0).filter(|id| ITEM_IDS.contains(id))?;
    let (count, bonus) = (u32_at(4)?, u32_at(8)?);
    // Sanity: a round brings a handful at most.
    if count > 1000 || bonus > 1000 {
        return None;
    }
    let count = if outcome == Outcome::Failed { 0 } else { count + bonus };
    Some(Gathered { node: node.value, outcome, item, count, bonus })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// The frame as the parser gets it: the length varint (body + 3) first.
    fn frame(body: &str) -> Vec<u8> {
        let body = hex(body);
        let mut f = vec![(body.len() + 3) as u8];
        f.extend(body);
        f
    }

    #[test]
    fn rounds_of_the_capture() {
        // Plain, better with a bonus, failed (2026-11-10).
        assert_eq!(
            read(&frame("338D003A87C801BAEE652401000000000000000000000000000000")),
            Some(Gathered { node: 25607, outcome: Outcome::Plain, item: 610_660_026, count: 1, bonus: 0 })
        );
        assert_eq!(
            read(&frame("338D013AD8F5025E2B652401000000010000000000000000000000")),
            Some(Gathered { node: 47832, outcome: Outcome::Better, item: 610_610_014, count: 2, bonus: 1 })
        );
        assert_eq!(
            read(&frame("338D023A91DA01BAEE652400000000000000000000000000000000")),
            Some(Gathered { node: 27921, outcome: Outcome::Failed, item: 610_660_026, count: 0, bonus: 0 })
        );
    }

    #[test]
    fn not_a_round() {
        assert_eq!(read(&frame("348DA079023A91DA01")), None);
        assert_eq!(read(&frame("338D073A91DA01BAEE652400000000000000000000000000000000")), None);
        assert_eq!(read(&frame("338D02")), None);
    }
}
