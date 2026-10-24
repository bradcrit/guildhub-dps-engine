//! What the player looted: the reward list of a cube opened (`15 57`), and
//! single items added to the inventory (`1B 56`). Read 2026-10-24 on a Fire
//! Temple run (instance 600022): the cube after Kromede's Desire, and the
//! boss souls and Odyle picked up before it, matched what the game showed.
//!
//! ```text
//! 15 57 00 00 <kind u8: 2 seen> <n varint>
//!         n x ( <item u32> <count u32> )
//! 1B 56 <n u8> <u8> <u8>
//!         n x ( <stack u32> 00 <slot u16> 00 <item u32> <count u64> … )
//! ```
//!
//! - The cube's list is the whole reward at once, Kinah and Enhance Stones
//!   included. Its items then arrive as one `1B 56` with every stack it
//!   changed (n = 27 for a 21-item list), whose entries run to different
//!   lengths (gear carries its rolls) — not read as loot, only to keep the
//!   stack counts below current.
//! - A `1B 56` with one entry is a stack changed: `count` is the stack's
//!   count now. The second byte is 0 for a stack just made (a boss soul, the
//!   first Odyle), 1 for one that was there (the second). A gain is the
//!   count over what the stack held when last seen; a stack first seen
//!   already there is only remembered — the inventory isn't read at login.
//! - Item ids are the game's own (aion2.app's: 610610014 Fine Odyle).
//! - Only the player's own inventory comes this way: nothing here says what
//!   anyone else in the party got.

use std::collections::{HashMap, HashSet};

const CUBE: [u8; 2] = [0x15, 0x57];
const INVENTORY: [u8; 2] = [0x1B, 0x56];
/// A cube's list, as one entry of it: item and count, u32 each.
const CUBE_ENTRY: usize = 8;
const MAX_CUBE_ITEMS: u64 = 128;
/// A one-entry inventory frame's body (after the type).
const SINGLE_BODY: usize = 41;
/// Item ids are nine digits.
const ITEM_IDS: std::ops::RangeInclusive<u32> = 100_000_000..=999_999_999;
/// Stacks remembered at most; past it the oldest-seen are dropped (a gain on
/// one is then missed once).
const MAX_STACKS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LootSource {
    /// The reward list of a cube (or chest) opened.
    Cube,
    /// An item that went into the inventory on its own: a boss's soul, a
    /// gathered Odyle, Kinah from a mob.
    Item,
}

impl LootSource {
    pub fn as_str(self) -> &'static str {
        match self {
            LootSource::Cube => "cube",
            LootSource::Item => "item",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Loot {
    pub source: LootSource,
    /// (item id, count), in the game's order.
    pub items: Vec<(u32, u64)>,
}

/// Reads loot off the frames, keeping the stack counts it needs to tell a
/// gain from a change.
#[derive(Default)]
pub struct LootReader {
    /// Stack id -> (item, count) as last seen, and when (a counter).
    stacks: HashMap<u32, (u32, u64, u64)>,
    seen: u64,
    /// Items the last cube listed: their next stack change is the cube's,
    /// already counted — remembered, not counted again.
    from_cube: HashSet<u32>,
}

impl LootReader {
    pub fn new() -> Self {
        Self::default()
    }

    /// A whole frame (its length varint, type, body): the loot it says, if any.
    pub fn read(&mut self, frame: &[u8]) -> Option<Loot> {
        let (_, at) = varint(frame, 0)?;
        let kind = frame.get(at..at + 2)?;
        let body = &frame[at + 2..];
        if kind == CUBE {
            let items = cube_items(body)?;
            self.from_cube = items.iter().map(|&(item, _)| item).collect();
            return Some(Loot { source: LootSource::Cube, items });
        }
        if kind != INVENTORY {
            return None;
        }
        if body.first() == Some(&1) && body.len() == SINGLE_BODY {
            let (stack, item, count) = entry_at(body, 3)?;
            let gain = self.gain(stack, item, count, body[1] == 0);
            return (gain > 0).then(|| Loot { source: LootSource::Item, items: vec![(item, gain)] });
        }
        if body.first().is_some_and(|&n| n > 1) {
            self.refresh(body);
        }
        None
    }

    /// How much `stack` grew to `count`; a new stack's whole count. Remembers it.
    fn gain(&mut self, stack: u32, item: u32, count: u64, new: bool) -> u64 {
        let before = self.stacks.get(&stack).filter(|(was, ..)| *was == item).map(|&(_, n, _)| n);
        self.remember(stack, item, count);
        if self.from_cube.remove(&item) {
            return 0;
        }
        match before {
            Some(n) => count.saturating_sub(n),
            None if new => count,
            None => 0,
        }
    }

    fn remember(&mut self, stack: u32, item: u32, count: u64) {
        if self.stacks.len() >= MAX_STACKS && !self.stacks.contains_key(&stack) {
            if let Some(oldest) = self.stacks.iter().min_by_key(|&(_, &(.., seen))| seen).map(|(&k, _)| k) {
                self.stacks.remove(&oldest);
            }
        }
        self.seen += 1;
        self.stacks.insert(stack, (item, count, self.seen));
    }

    /// A frame of several stacks: the counts of the ones known, wherever an
    /// entry for them reads (entries differ in length, so each is looked for).
    fn refresh(&mut self, body: &[u8]) {
        let mut found = Vec::new();
        for (&stack, &(item, ..)) in &self.stacks {
            let key = stack.to_le_bytes();
            let mut from = 0;
            while let Some(i) = body.get(from..).and_then(|rest| rest.windows(4).position(|w| w == key)) {
                let at = from + i;
                if let Some((s, it, count)) = entry_at(body, at) {
                    if s == stack && it == item {
                        found.push((stack, item, count));
                        break;
                    }
                }
                from = at + 1;
            }
        }
        for (stack, item, count) in found {
            self.remember(stack, item, count);
        }
    }
}

/// A cube's list: `00 00 <kind> <n varint>` and exactly n entries.
fn cube_items(body: &[u8]) -> Option<Vec<(u32, u64)>> {
    let (n, at) = varint(body, 3)?;
    if n == 0 || n > MAX_CUBE_ITEMS || body.len() != at + n as usize * CUBE_ENTRY {
        return None;
    }
    body[at..]
        .chunks_exact(CUBE_ENTRY)
        .map(|e| {
            let item = u32::from_le_bytes(e[0..4].try_into().ok()?);
            let count = u32::from_le_bytes(e[4..8].try_into().ok()?);
            (ITEM_IDS.contains(&item) && count > 0).then_some((item, u64::from(count)))
        })
        .collect()
}

/// An inventory entry at `at`: (stack, item, count), if it reads as one.
fn entry_at(body: &[u8], at: usize) -> Option<(u32, u32, u64)> {
    let e = body.get(at..at + 23)?;
    if e[4] != 0 || e[7] != 0 {
        return None;
    }
    let stack = u32::from_le_bytes(e[0..4].try_into().ok()?);
    let item = u32::from_le_bytes(e[8..12].try_into().ok()?);
    let count = u64::from_le_bytes(e[12..20].try_into().ok()?);
    (ITEM_IDS.contains(&item)).then_some((stack, item, count))
}

fn varint(b: &[u8], mut i: usize) -> Option<(u64, usize)> {
    let mut v = 0u64;
    let mut shift = 0;
    loop {
        let x = *b.get(i)?;
        i += 1;
        v |= u64::from(x & 0x7F) << shift;
        if x & 0x80 == 0 {
            return Some((v, i));
        }
        shift += 7;
        if shift > 35 {
            return None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        let s = s.trim();
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    const FINE_ODYLE_NEW: &str = "2F1B56010000D8DD120000BB00005E2B65240100000000000000013B00000000000000000000000000290000";
    const ODYLE_NEW: &str = "2F1B560100005811180000B200005D2B65240100000000000000013C00000000000000000000000000290000";
    const FINE_ODYLE_TWO: &str = "2F1B56010100D8DD120000BB00005E2B65240200000000000000013B00000000000000000000000000290000";
    const KINAH_BEFORE: &str = "2F1B56010100D2190F00009B01003F3370370777AA00000000000100000000000000000000000000000B0000";
    const KINAH_AFTER: &str = "2F1B56010100D2190F00009B01003F3370371777AA00000000000100000000000000000000000000000B0000";

    /// The cube after Kromede's Desire, 2026-10-24: 21 entries, as the game listed them.
    #[test]
    fn the_cube_lists_the_whole_reward() {
        let loot = LootReader::new().read(&hex(include_str!("testdata/loot_cube.hex"))).unwrap();
        assert_eq!(loot.source, LootSource::Cube);
        assert_eq!(loot.items.len(), 21);
        assert_eq!(loot.items[0], (930100031, 800_000)); // Kinah
        assert_eq!(loot.items[2], (110140044, 1)); // Corrupted Judicator Greatsword
        assert!(loot.items.contains(&(630840002, 14))); // Silentium
    }

    /// New stacks count whole, a known one by what it grew.
    #[test]
    fn picked_up_items_count_what_they_added() {
        let mut r = LootReader::new();
        assert_eq!(r.read(&hex(FINE_ODYLE_NEW)).unwrap().items, vec![(610610014, 1)]);
        assert_eq!(r.read(&hex(ODYLE_NEW)).unwrap().items, vec![(610610013, 1)]);
        assert_eq!(r.read(&hex(FINE_ODYLE_TWO)).unwrap().items, vec![(610610014, 1)]);
    }

    /// A stack first seen already there is only remembered: Kinah held is no loot.
    #[test]
    fn a_stack_held_before_is_not_loot() {
        let mut r = LootReader::new();
        assert_eq!(r.read(&hex(KINAH_BEFORE)), None);
        assert_eq!(r.read(&hex(KINAH_AFTER)).unwrap().items, vec![(930100031, 16)]);
    }

    /// The cube's Kinah isn't counted again when the stack next changes.
    #[test]
    fn the_cube_is_not_counted_twice() {
        let mut r = LootReader::new();
        r.read(&hex(KINAH_BEFORE));
        r.read(&hex(include_str!("testdata/loot_cube.hex")));
        assert_eq!(r.read(&hex(include_str!("testdata/loot_inventory_after_cube.hex"))), None);
        assert_eq!(r.read(&hex(KINAH_AFTER)), None);
        assert_eq!(r.read(&hex(FINE_ODYLE_NEW)).unwrap().items, vec![(610610014, 1)]);
    }

    /// A frame of several stacks updates the counts of the known ones.
    #[test]
    fn a_frame_of_several_stacks_keeps_counts_current() {
        let mut r = LootReader::new();
        // The cube's Murute Earrings stack (1223727), as if seen before at 0.
        r.remember(1223727, 310240048, 0);
        assert_eq!(r.read(&hex(include_str!("testdata/loot_inventory_after_cube.hex"))), None);
        assert_eq!(r.stacks[&1223727].1, 1);
    }

    #[test]
    fn other_frames_are_no_loot() {
        assert_eq!(LootReader::new().read(&hex("0E0036719301 6DA1010000".replace(' ', "").as_str())), None);
    }
}
