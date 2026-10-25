//! An entity's resources — HP, MP, stamina, flight time and their maxima —
//! for GuildHub Companion's resource bars.
//!
//! ```text
//! <len> 00 8d <entity> <lists u8>
//!             [lists & 1: <n u8> n × (<stat u8> <value u32>)]
//!             [lists & 2: <n u8> n × (<stat u8> <value u64>)]
//! ```
//!
//! The stats seen (captures of 2026-10-25, 1331 of your own records, all
//! read to their last byte; the maxima from the login record):
//!
//! | stat | list | what            | max stat |
//! |------|------|-----------------|----------|
//! | 00   | u64  | HP              | 07       |
//! | 01   | u32  | MP              | 08       |
//! | 03   | u32  | stamina         | 0a       |
//! | 06   | u32  | flight time     | 0d       |
//!
//! A stat's maximum is the stat 7 above it, in the same list. Monsters send
//! only the u64 list, their HP (what `scan_for_entity_hp` reads); your own
//! character sends the u32 list as it changes, every 100 ms while sprinting or
//! flying: stamina and flight time fall then and come back on their own,
//! flight time also by an item (+200,000 at once). Stamina, flight time and
//! their maxima were seen in the captures; MP is the u32 stat that pairs with
//! max stat 08 (2305, then 4099 after the login's second record), not yet seen
//! changing in one.

const OPCODE: [u8; 2] = [0x00, 0x8D];

pub const HP: u8 = 0x00;
pub const MP: u8 = 0x01;
pub const STAMINA: u8 = 0x03;
pub const FLIGHT: u8 = 0x06;
/// A stat's maximum is this far above it.
pub const MAX_OFFSET: u8 = 7;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Update {
    pub entity: i32,
    /// The u32 list's stats.
    pub small: Vec<(u8, u32)>,
    /// The u64 list's stats.
    pub wide: Vec<(u8, u64)>,
}

/// One `00 8d` record at the front of `packet`, length varint included.
/// None for anything else, and for a record that doesn't end where its lists
/// do — the same opcode pair turns up in other shapes.
pub fn parse(packet: &[u8]) -> Option<Update> {
    let length = super::stream_processor::read_varint(packet, 0);
    if length.length <= 0 {
        return None;
    }
    let mut at = length.length as usize;
    if packet.get(at..at + 2)? != OPCODE {
        return None;
    }
    at += 2;
    let entity = super::stream_processor::read_varint(packet, at);
    if entity.length <= 0 || !(100..=9_999_999).contains(&entity.value) {
        return None;
    }
    at += entity.length as usize;
    let lists = *packet.get(at)?;
    at += 1;
    if lists == 0 || lists & !3 != 0 {
        return None;
    }
    let mut small = Vec::new();
    if lists & 1 != 0 {
        let n = *packet.get(at)? as usize;
        at += 1;
        for _ in 0..n {
            let b = packet.get(at..at + 5)?;
            small.push((b[0], u32::from_le_bytes([b[1], b[2], b[3], b[4]])));
            at += 5;
        }
    }
    let mut wide = Vec::new();
    if lists & 2 != 0 {
        let n = *packet.get(at)? as usize;
        at += 1;
        for _ in 0..n {
            let b = packet.get(at..at + 9)?;
            wide.push((b[0], u64::from_le_bytes(b[1..9].try_into().ok()?)));
            at += 9;
        }
    }
    (at == packet.len() && (!small.is_empty() || !wide.is_empty())).then_some(Update { entity: entity.value, small, wide })
}

/// What is known of one entity's resources: each stat's last value.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resources {
    small: std::collections::HashMap<u8, u32>,
    wide: std::collections::HashMap<u8, u64>,
}

/// A resource now and at most, as the bars show it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gauge {
    pub current: u64,
    pub max: u64,
}

impl Resources {
    pub fn note(&mut self, update: &Update) {
        self.small.extend(update.small.iter().copied());
        self.wide.extend(update.wide.iter().copied());
    }

    /// A u32 stat (MP, stamina, flight time) with its maximum, None until
    /// the maximum was heard of. The value comes only as it changes: full
    /// until then, and never above the maximum.
    pub fn small(&self, stat: u8) -> Option<Gauge> {
        let max = self.small.get(&(stat + MAX_OFFSET)).map(|&m| m as u64).filter(|&m| m > 0)?;
        let current = self.small.get(&stat).map_or(max, |&v| v as u64);
        Some(Gauge { current: current.min(max), max })
    }

    /// A u64 stat (HP), as `small`.
    pub fn wide(&self, stat: u8) -> Option<Gauge> {
        let max = self.wide.get(&(stat + MAX_OFFSET)).copied().filter(|&m| m > 0)?;
        let current = self.wide.get(&stat).copied().unwrap_or(max);
        Some(Gauge { current: current.min(max), max })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        let s: String = s.split_whitespace().collect();
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// A record of a capture, framed: the length varint counts the payload plus 4.
    fn framed(payload: &str) -> Vec<u8> {
        let body = hex(payload);
        let mut out = vec![(body.len() + 4) as u8];
        out.extend(body);
        out
    }

    /// The login's two records (2026-10-25 17:50:43, entity 14159): six u32
    /// maxima — max MP 08, max stamina 0a, max flight time 0d among them —
    /// and max HP 07 in the u64 list; 150 ms later max MP and max HP again,
    /// the gear's stats on them.
    #[test]
    fn the_login_records_carry_the_maxima() {
        let first = parse(&framed(
            "00 8d cf 6e 03 06 04 20 03 00 00 08 01 09 00 00 0a a0 35 02 00 0b f0 49 02 00 0c a0 86 01 00 0d a5 25 0a 00 01 07 28 35 00 00 00 00 00 00",
        ))
        .unwrap();
        assert_eq!(first.entity, 14159);
        assert_eq!(first.small, vec![(0x04, 800), (0x08, 2305), (0x0a, 144_800), (0x0b, 150_000), (0x0c, 100_000), (0x0d, 664_997)]);
        assert_eq!(first.wide, vec![(0x07, 13_608)]);
        let second = parse(&framed("00 8d cf 6e 03 01 08 03 10 00 00 01 07 6d 61 00 00 00 00 00 00")).unwrap();
        assert_eq!(second.small, vec![(0x08, 4099)]);
        assert_eq!(second.wide, vec![(0x07, 24_941)]);

        let mut r = Resources::default();
        r.note(&first);
        r.note(&second);
        // Full until they first change.
        assert_eq!(r.wide(HP), Some(Gauge { current: 24_941, max: 24_941 }));
        assert_eq!(r.small(MP), Some(Gauge { current: 4099, max: 4099 }));
        assert_eq!(r.small(0x02), None, "no maximum heard of");
    }

    /// Sprinting (17:50:53 on): stamina 03 falls by 500 every 100 ms; both
    /// stamina and flight time in one record at 17:51:04.
    #[test]
    fn stamina_and_flight_time_as_they_change() {
        let mut r = Resources::default();
        r.note(&parse(&framed("00 8d cf 6e 03 06 04 20 03 00 00 08 01 09 00 00 0a a0 35 02 00 0b f0 49 02 00 0c a0 86 01 00 0d a5 25 0a 00 01 07 28 35 00 00 00 00 00 00")).unwrap());
        let both = parse(&framed("00 8d cf 6e 01 02 03 e8 d0 01 00 06 a3 08 01 00")).unwrap();
        assert_eq!(both.small, vec![(STAMINA, 119_016), (FLIGHT, 67_747)]);
        r.note(&both);
        assert_eq!(r.small(STAMINA), Some(Gauge { current: 119_016, max: 144_800 }));
        assert_eq!(r.small(FLIGHT), Some(Gauge { current: 67_747, max: 664_997 }));
        r.note(&parse(&framed("00 8d cf 6e 01 01 06 f7 00 00 00")).unwrap());
        assert_eq!(r.small(FLIGHT).unwrap().current, 247);
    }

    /// A monster's HP (`02 01 00 <u64>`, as `scan_for_entity_hp` reads it) is
    /// the u64 list's stat 00.
    #[test]
    fn a_monsters_hp_is_the_wide_list() {
        let update = parse(&framed("00 8d ef c9 01 02 01 00 df 12 00 00 00 00 00 00")).unwrap();
        assert_eq!(update.entity, 25839);
        assert_eq!(update.wide, vec![(HP, 0x12df)]);
    }

    #[test]
    fn other_shapes_are_not_read() {
        // Cut short, one byte too many, an unknown list flag, another opcode.
        assert_eq!(parse(&framed("00 8d cf 6e 01 01 06 f7 00 00")), None);
        assert_eq!(parse(&framed("00 8d cf 6e 01 01 06 f7 00 00 00 00")), None);
        assert_eq!(parse(&framed("00 8d cf 6e 04 01 06 f7 00 00 00")), None);
        assert_eq!(parse(&framed("02 8d cf 6e d0 07 00 00")), None);
    }
}
