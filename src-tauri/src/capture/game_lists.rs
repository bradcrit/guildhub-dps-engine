//! Your entry counts and currencies, as the game sends them.
//!
//! **Entry counts** (`0B 61` the whole list at login, `0C 61` the entries that
//! changed): Nightmare, Shugo Festival keys, Abyss time, Odyle energy … one
//! list of `<kind u8> <id u32 LE> <values>`. The kind says which values follow,
//! a zero being left out: `01` a time in ms (u64 LE), `04` the first number,
//! `08` the second, `0c` both (varints), `00` none. What the ids are, as far as
//! read from the game's own windows (2026-10-18):
//!
//! | id         | values                                       |
//! |------------|----------------------------------------------|
//! | 1          | Shugo Festival keys (left, extras)           |
//! | 3          | Lower Reshanta basic usage time left (ms)    |
//! | 4          | Nightmare (left, extras)                     |
//! | 60000001   | Odyle energy (basic, charged)                |
//!
//! The list at 00:55 on 2026-10-18 had 73 entries and ended exactly where the
//! packet did, which is what says the layout is right; the ones above changed
//! with the game's windows (Nightmare 0(+9) → 0(+8) after a run).
//!
//! **Currencies** are items in your inventory (`11 56` the whole of it at login,
//! `1B 56` the items whose amount changed): `<u32> <u32> <item id u32> <amount
//! u64> …`. The first two differ from login to login, the item id is the
//! game's, the same for every character. Only the ones in `CURRENCIES` are
//! looked for, by their id.
//!
//! **Duty missions** (`1C 8D`, the Journal's list, at login and after each
//! reward taken): `<list u32> <done today u32> <count u8> <missions …>`. List 1
//! is Duty → Mission, its "done today" the Journal's "Per Server Daily
//! Completion" (0/5 → 1/5 → 2/5 as rewards were taken, 2026-10-25). Other
//! lists (0) come when the Journal opens; only list 1 is read.
//!
//! All of these are your own character's only — the game sends nobody else's.

/// The currencies looked for: the game's item id, and the name the engine
/// gives it.
///
/// The game's top bar shows Kina and, before it, Kina plus Kina (Bound): one
/// character had 1,920 and 319,600 here, the bar 1,920 and 321,520.
pub const CURRENCIES: &[(u32, &str)] = &[
    (930_100_031, "kinah"),
    (930_100_001, "kinahBound"),
    (930_100_003, "abyssPoints"),
    (930_100_017, "nightmareCoins"),
    (930_100_011, "shugoCoins"),
];

/// One entry count: its numbers (0 when left out) and its time, if it has one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntryCount {
    pub id: u32,
    pub first: u32,
    pub second: u32,
    pub ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListUpdate {
    /// Entry counts; `whole`: the list at login, anything not in it is gone.
    Counts { entries: Vec<EntryCount>, whole: bool },
    /// Currencies as (item id, amount); `whole`: the inventory at login, a
    /// currency not in it is at 0.
    Items { items: Vec<(u32, u64)>, whole: bool },
    /// Duty missions whose reward was taken today.
    Duties { done: u32 },
}

/// The Journal's list of Duty → Mission (`1C 8D`'s first number).
const DUTY_MISSIONS: u32 = 1;

pub fn parse(packet: &[u8]) -> Option<ListUpdate> {
    let length = super::stream_processor::read_varint(packet, 0);
    if length.length <= 0 {
        return None;
    }
    let at = length.length as usize;
    let body = packet.get(at + 2..)?;
    match packet.get(at..at + 2)? {
        [0x0B, 0x61] => Some(ListUpdate::Counts { entries: whole_list(body)?, whole: true }),
        [0x0C, 0x61] => Some(ListUpdate::Counts { entries: changed(body)?, whole: false }),
        [0x11, 0x56] => Some(ListUpdate::Items { items: currencies_in(body), whole: true }),
        [0x1B, 0x56] => {
            let items = currencies_in(body);
            (!items.is_empty()).then_some(ListUpdate::Items { items, whole: false })
        }
        [0x1C, 0x8D] => {
            let list = u32::from_le_bytes(body.get(0..4)?.try_into().ok()?);
            let done = u32::from_le_bytes(body.get(4..8)?.try_into().ok()?);
            // Generous: only to turn away a list that isn't the one.
            (list == DUTY_MISSIONS && done <= 100).then_some(ListUpdate::Duties { done })
        }
        _ => None,
    }
}

fn varint(body: &[u8], at: &mut usize) -> Option<u32> {
    let v = super::stream_processor::read_varint(body, *at);
    if v.length <= 0 || v.value < 0 {
        return None;
    }
    *at += v.length as usize;
    Some(v.value as u32)
}

/// The entry at `at`, and where the next begins.
fn entry_at(body: &[u8], at: usize) -> Option<(EntryCount, usize)> {
    let kind = *body.get(at)?;
    // A time together with numbers was never seen: in which order they'd
    // come is unknown, so such an entry (and the list) is not read.
    if !matches!(kind, 0x00 | 0x01 | 0x04 | 0x08 | 0x0C) {
        return None;
    }
    let id = u32::from_le_bytes(body.get(at + 1..at + 5)?.try_into().ok()?);
    let mut o = at + 5;
    let mut entry = EntryCount { id, first: 0, second: 0, ms: None };
    if kind & 0x01 != 0 {
        entry.ms = Some(u64::from_le_bytes(body.get(o..o + 8)?.try_into().ok()?));
        o += 8;
    }
    if kind & 0x04 != 0 {
        entry.first = varint(body, &mut o)?;
    }
    if kind & 0x08 != 0 {
        entry.second = varint(body, &mut o)?;
    }
    Some((entry, o))
}

/// `0b 61 <count varint> <entries>`: all of it or nothing — a list read only
/// in part would drop the rest's counts.
fn whole_list(body: &[u8]) -> Option<Vec<EntryCount>> {
    let mut at = 0;
    let count = varint(body, &mut at)? as usize;
    if count == 0 || count > 1000 {
        return None;
    }
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let (entry, next) = entry_at(body, at)?;
        entries.push(entry);
        at = next;
    }
    Some(entries)
}

/// `0c 61 <n> <entries> …`: `n` is not always the count — `00` comes with one
/// entry too (see `StreamProcessor::parse_odyle_energy_packet`) — and a byte
/// or so follows the entries. Read: at least one, as many as `n` says.
fn changed(body: &[u8]) -> Option<Vec<EntryCount>> {
    let n = (*body.first()?).max(1);
    let mut at = 1;
    let mut entries = Vec::new();
    for _ in 0..n {
        let Some((entry, next)) = entry_at(body, at) else { break };
        entries.push(entry);
        at = next;
    }
    (!entries.is_empty()).then_some(entries)
}

/// Each of `CURRENCIES` found in an inventory record: its id, then the amount.
fn currencies_in(body: &[u8]) -> Vec<(u32, u64)> {
    CURRENCIES
        .iter()
        .filter_map(|&(id, _)| {
            let needle = id.to_le_bytes();
            let at = body.windows(4).position(|w| w == needle)?;
            let amount = u64::from_le_bytes(body.get(at + 4..at + 12)?.try_into().ok()?);
            // Generous: only to turn away bytes that merely look like the record.
            (amount < 1_000_000_000_000).then_some((id, amount))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(hex: &str) -> Vec<u8> {
        hex.split_whitespace().map(|b| u8::from_str_radix(b, 16).unwrap()).collect()
    }

    fn packet(opcode: &str, body: &str) -> Vec<u8> {
        let mut inner = bytes(opcode);
        inner.extend(bytes(body));
        // A one-byte length is enough for these; the parser only skips it.
        let mut p = vec![0x05];
        p.extend(inner);
        p
    }

    /// The start of the login list of 2026-10-18 00:55, cut to four entries.
    #[test]
    fn a_login_list_is_read_whole() {
        let p = packet("0B 61", "04 04 01 00 00 00 13 01 03 00 00 00 F0 23 2C 01 00 00 00 00 08 04 00 00 00 09 0C 01 87 93 03 0F 91 0B");
        let Some(ListUpdate::Counts { entries, whole: true }) = parse(&p) else { panic!("{:?}", parse(&p)) };
        assert_eq!(entries[0], EntryCount { id: 1, first: 19, second: 0, ms: None });
        assert_eq!(entries[1], EntryCount { id: 3, first: 0, second: 0, ms: Some(19_670_000) });
        assert_eq!(entries[2], EntryCount { id: 4, first: 0, second: 9, ms: None });
        assert_eq!(entries[3], EntryCount { id: 60_000_001, first: 15, second: 1425, ms: None });
    }

    #[test]
    fn a_list_that_runs_out_is_not_read() {
        assert_eq!(parse(&packet("0B 61", "05 04 01 00 00 00 13 08 04 00 00 00 09")), None);
    }

    /// After a Nightmare run, 2026-10-18 00:58:08.
    #[test]
    fn a_change_with_00_in_front_carries_its_entry() {
        let Some(ListUpdate::Counts { entries, whole: false }) = parse(&packet("0C 61", "00 08 04 00 00 00 08 03")) else { panic!() };
        assert_eq!(entries, vec![EntryCount { id: 4, first: 0, second: 8, ms: None }]);
    }

    /// Abyss points 82,325 → 72,325 for a Stigma shard, 2026-10-18 00:56:30.
    #[test]
    fn a_currency_is_found_by_its_item_id() {
        let p = packet("1B 56", "02 01 00 BA 8E 13 00 00 8A 01 00 23 33 70 37 85 1A 01 00 00 00 00 00 01 02 00 00 00 00 00 00 00 00 00 00 00 00 00 00 0F 00 00 00 B2 D6 16 00 00 09 01 00 6A 26 B4 1F 01 00 00 00 00 00 00 00");
        assert_eq!(parse(&p), Some(ListUpdate::Items { items: vec![(930_100_003, 72_325)], whole: false }));
    }

    /// After the second Duty reward of the day, 2026-10-25 13:32:32 (cut after the first mission).
    #[test]
    fn the_duty_list_says_how_many_are_done() {
        let p = packet("1C 8D", "01 00 00 00 02 00 00 00 11 0B 01 00 00 3A 20 23 00 29 01 00 00");
        assert_eq!(parse(&p), Some(ListUpdate::Duties { done: 2 }));
    }

    /// The Journal's other list, as it opened at 2026-10-25 13:23:49.
    #[test]
    fn another_journal_list_is_nothing() {
        let p = packet("1C 8D", "00 00 00 00 00 00 00 00 0F 00 01 00 00 C7 20 23 00");
        assert_eq!(parse(&p), None);
    }

    #[test]
    fn an_item_change_without_a_currency_is_nothing() {
        let p = packet("1B 56", "01 01 00 A6 9B 12 00 00 5B 01 00 21 F4 CB 1F 34 50 00 00 00 00 00 00 01 26");
        assert_eq!(parse(&p), None);
    }
}
