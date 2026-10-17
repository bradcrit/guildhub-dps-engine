//! Hunting for the record that carries a number the game shows.
//!
//! Nightmare charges, daily dungeon entries, Abyss points: the game shows them,
//! so the server sends them, but we don't know in which packet. What we do know
//! is how the number moved while the capture ran — 14 Nightmare charges, one
//! run, 13. This walks every packet of a capture and reports the fields (an
//! opcode, a position in it, a width) that held exactly those values, in that
//! order. A field that read 14 and later 13 in a packet of one opcode, at one
//! position, and never anything else, is almost always the record.
//!
//! Two passes: the first notes only where a value of the trail appears, the
//! second follows those candidates through the whole capture to see what else
//! they held — a position that also read 0, 7 and 200 is a coincidence.
//!
//!   A2_REPLAY_CAPTURE=<packets_….txt or a folder of them> A2_VALUES=14,13 \
//!     cargo test --release --test find_value_packets -- --ignored --nocapture
//!
//! `A2_OPCODE=0D 90` dumps every packet of one opcode with its time instead,
//! to read a record found by the first run.

use std::collections::{HashMap, HashSet};

use a2tools_dps_meter_lib::capture::framing::{self, FrameKind};
use a2tools_dps_meter_lib::capture::packet_accumulator::PacketAccumulator;
use a2tools_dps_meter_lib::capture::stream_processor::read_varint;
use a2tools_dps_meter_lib::share;

/// How far into a packet's body a field is looked for, from either end.
const MAX_OFFSET: usize = 512;
/// Field widths in bytes; 0 is a varint (from the start only).
const WIDTHS: [usize; 5] = [0, 1, 2, 4, 8];
/// A candidate that changed more often than this is a counter or a timer.
const MAX_CHANGES: usize = 40;

/// A packet: when it was captured, its opcode, the bytes after the opcode.
struct Packet {
    at_ms: i64,
    opcode: [u8; 2],
    body: Vec<u8>,
}

/// Where a field sits: from the start of the body or back from its end
/// (records with a variable-length head keep their tail in place).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Anchor {
    Start,
    End,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Field {
    opcode: [u8; 2],
    anchor: Anchor,
    offset: usize,
    width: usize,
}

impl Field {
    fn read(&self, body: &[u8]) -> Option<u64> {
        if self.width == 0 {
            let v = read_varint(body, self.offset);
            return (self.anchor == Anchor::Start && v.length > 0 && v.value >= 0).then_some(v.value as u64);
        }
        let start = match self.anchor {
            Anchor::Start => self.offset,
            Anchor::End => body.len().checked_sub(self.offset + self.width)?,
        };
        let bytes = body.get(start..start + self.width)?;
        let mut v = [0u8; 8];
        v[..self.width].copy_from_slice(bytes);
        Some(u64::from_le_bytes(v))
    }
}

/// The capture files: one, or every `packets_*.txt` in a folder, oldest first.
fn capture_files(path: &str) -> Vec<std::path::PathBuf> {
    let path = std::path::Path::new(path);
    if !path.is_dir() {
        return vec![path.to_path_buf()];
    }
    let mut files: Vec<_> = std::fs::read_dir(path)
        .expect("capture folder")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("packets_") && n.ends_with(".txt")))
        .collect();
    files.sort();
    files
}

/// Every plain packet, bundles decompressed, streams reassembled, with the
/// time of the capture line it completed on.
fn all_packets(path: &str) -> Vec<Packet> {
    fn push(out: &mut Vec<Packet>, at_ms: i64, packet: &[u8]) {
        let li = read_varint(packet, 0);
        if li.length <= 0 {
            return;
        }
        let o = li.length as usize;
        if o + 2 > packet.len() {
            return;
        }
        out.push(Packet { at_ms, opcode: [packet[o], packet[o + 1]], body: packet[o + 2..].to_vec() });
    }
    fn inner(buf: &[u8], at_ms: i64, out: &mut Vec<Packet>, depth: usize) {
        if depth > 4 {
            return;
        }
        for f in framing::walk_inner(buf).frames {
            match f.kind {
                FrameKind::Packet => push(out, at_ms, f.bytes(buf)),
                FrameKind::Bundle => {
                    if let Some(d) = framing::decompress_bundle(f.payload(buf)) {
                        inner(&d, at_ms, out, depth + 1);
                    }
                }
            }
        }
    }

    let mut out = Vec::new();
    let mut streams: HashMap<String, PacketAccumulator> = HashMap::new();
    for file in capture_files(path) {
        for cap in share::read_capture(&file).expect("capture") {
            let acc = streams.entry(cap.stream.clone()).or_insert_with(PacketAccumulator::new);
            acc.append(&cap.bytes);
            let buf = acc.snapshot().to_vec();
            let walk = framing::walk(&buf);
            for f in &walk.frames {
                match f.kind {
                    FrameKind::Packet => push(&mut out, cap.captured_at_ms, f.bytes(&buf)),
                    FrameKind::Bundle => {
                        if let Some(d) = framing::decompress_bundle(f.payload(&buf)) {
                            inner(&d, cap.captured_at_ms, &mut out, 1);
                        }
                    }
                }
            }
            acc.discard_bytes(walk.consumed);
        }
    }
    out
}

fn clock(at_ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local.timestamp_millis_opt(at_ms).single().map(|t| t.format("%H:%M:%S%.3f").to_string()).unwrap_or_default()
}

fn hex(bytes: &[u8], max: usize) -> String {
    let shown: Vec<String> = bytes.iter().take(max).map(|b| format!("{b:02X}")).collect();
    let more = if bytes.len() > max { " …" } else { "" };
    format!("{}{more}", shown.join(" "))
}

fn parse_opcode(text: &str) -> Option<[u8; 2]> {
    let bytes: Vec<u8> = text.split_whitespace().filter_map(|b| u8::from_str_radix(b, 16).ok()).collect();
    (bytes.len() == 2).then(|| [bytes[0], bytes[1]])
}

/// The values a field held over time, each run of one value once.
fn history(field: &Field, packets: &[Packet]) -> Vec<(i64, u64)> {
    let mut seen: Vec<(i64, u64)> = Vec::new();
    for p in packets.iter().filter(|p| p.opcode == field.opcode) {
        if let Some(v) = field.read(&p.body) {
            if seen.last().map(|(_, last)| *last) != Some(v) {
                seen.push((p.at_ms, v));
            }
        }
    }
    seen
}

/// Whether `trail` appears in `values` back to back.
fn holds_trail(values: &[u64], trail: &[u64]) -> bool {
    values.windows(trail.len()).any(|w| w == trail)
}

#[test]
#[ignore = "diagnostic"]
fn which_fields_held_these_values() {
    let Ok(capture) = std::env::var("A2_REPLAY_CAPTURE") else {
        eprintln!("A2_REPLAY_CAPTURE must be set — skipping");
        return;
    };
    let packets = all_packets(&capture);
    println!("capture: {} packets", packets.len());

    if let Some(opcode) = std::env::var("A2_OPCODE").ok().as_deref().and_then(parse_opcode) {
        for p in packets.iter().filter(|p| p.opcode == opcode) {
            println!("{}  {:>4} B  {}", clock(p.at_ms), p.body.len(), hex(&p.body, 4000));
        }
        return;
    }

    let Some(trail) = std::env::var("A2_VALUES")
        .ok()
        .map(|v| v.split(',').filter_map(|n| n.trim().parse::<u64>().ok()).collect::<Vec<_>>())
        .filter(|t| t.len() >= 2)
    else {
        eprintln!("A2_VALUES must list two or more values in the order the game showed them, e.g. 14,13 — skipping");
        return;
    };
    let wanted: HashSet<u64> = trail.iter().copied().collect();

    // Pass 1: every field that held a value of the trail at least once.
    let mut hits: HashMap<Field, HashSet<u64>> = HashMap::new();
    for p in &packets {
        let n = p.body.len();
        for &width in &WIDTHS {
            for offset in 0..n.min(MAX_OFFSET) {
                for anchor in [Anchor::Start, Anchor::End] {
                    if width == 0 && anchor == Anchor::End {
                        continue;
                    }
                    let field = Field { opcode: p.opcode, anchor, offset, width };
                    if let Some(v) = field.read(&p.body) {
                        if wanted.contains(&v) {
                            hits.entry(field).or_default().insert(v);
                        }
                    }
                }
            }
        }
    }
    let candidates: Vec<Field> = hits.into_iter().filter(|(_, seen)| seen.len() == wanted.len()).map(|(f, _)| f).collect();
    println!("trail {trail:?}: {} fields held all of its values", candidates.len());

    // Pass 2: the whole story of each candidate.
    let mut found: Vec<(bool, Field, Vec<(i64, u64)>)> = Vec::new();
    for field in candidates {
        let story = history(&field, &packets);
        if story.len() > MAX_CHANGES {
            continue;
        }
        let values: Vec<u64> = story.iter().map(|(_, v)| *v).collect();
        if holds_trail(&values, &trail) {
            found.push((values == trail, field, story));
        }
    }
    // Exact stories first, then the fewest changes; one line per opcode and
    // story (the same u8 also reads as the low byte of a u16 …).
    found.sort_by_key(|(exact, f, story)| (!*exact, story.len(), f.width, f.offset));
    let mut shown: HashSet<([u8; 2], Vec<u64>)> = HashSet::new();
    let mut lines = 0;
    for (exact, field, story) in &found {
        let values: Vec<u64> = story.iter().map(|(_, v)| *v).collect();
        if !shown.insert((field.opcode, values)) {
            continue;
        }
        let count = packets.iter().filter(|p| p.opcode == field.opcode).count();
        println!(
            "\n{} opcode {:02X} {:02X}  {:?}+{} u{}  ({count} packets of it)",
            if *exact { "EXACT" } else { "     " },
            field.opcode[0],
            field.opcode[1],
            field.anchor,
            field.offset,
            if field.width == 0 { "varint".to_string() } else { format!("u{}", field.width * 8) },
        );
        for (at, v) in story.iter().take(12) {
            println!("    {}  {v}", clock(*at));
        }
        if let Some(p) = packets.iter().find(|p| p.opcode == field.opcode) {
            println!("    e.g. {}", hex(&p.body, 96));
        }
        lines += 1;
        if lines >= 25 {
            break;
        }
    }
    if found.is_empty() {
        println!(
            "\nNo field held {trail:?} in that order. The game may send the number only \
             when its window opens, or as a varint — open the window in game before and \
             after the change, and capture from login."
        );
    }
}

/// The bytes a number can be sent as: little-endian u16/u32/u64 and a varint.
fn encodings(v: u64) -> Vec<(&'static str, Vec<u8>)> {
    let mut out = Vec::new();
    if v <= u16::MAX as u64 {
        out.push(("u16", (v as u16).to_le_bytes().to_vec()));
    }
    if v <= u32::MAX as u64 {
        out.push(("u32", (v as u32).to_le_bytes().to_vec()));
    }
    out.push(("u64", v.to_le_bytes().to_vec()));
    let (mut rest, mut varint) = (v, Vec::new());
    loop {
        let b = (rest & 0x7F) as u8;
        rest >>= 7;
        if rest == 0 {
            varint.push(b);
            break;
        }
        varint.push(b | 0x80);
    }
    out.push(("varint", varint));
    out
}

/// Where a number appears at all, at any position: for records whose fields
/// move (lists, a variable head), which the fixed-field search can't follow.
///
///   A2_REPLAY_CAPTURE=… A2_FIND=82325,72325 cargo test --release --test find_value_packets where_does -- --ignored --nocapture
#[test]
#[ignore = "diagnostic"]
fn where_does_this_number_appear() {
    let (Ok(capture), Ok(find)) = (std::env::var("A2_REPLAY_CAPTURE"), std::env::var("A2_FIND")) else {
        eprintln!("A2_REPLAY_CAPTURE and A2_FIND must both be set — skipping");
        return;
    };
    let packets = all_packets(&capture);
    for v in find.split(',').filter_map(|n| n.trim().parse::<u64>().ok()) {
        println!("\n{v}:");
        let mut lines = 0;
        for p in &packets {
            for (name, bytes) in encodings(v) {
                // A u64 holding a small number is mostly zeros: only the widest
                // match per spot, and u16/varint only when they are 2+ bytes.
                if bytes.len() < 2 {
                    continue;
                }
                if let Some(at) = p.body.windows(bytes.len()).position(|w| w == bytes.as_slice()) {
                    println!(
                        "  {}  {:02X} {:02X}  +{at} {name}  ({} B)  {}",
                        clock(p.at_ms),
                        p.opcode[0],
                        p.opcode[1],
                        p.body.len(),
                        hex(&p.body[at.saturating_sub(16)..], 64)
                    );
                    lines += 1;
                }
            }
            if lines >= 40 {
                break;
            }
        }
        if lines == 0 {
            println!("  nowhere");
        }
    }
}

/// The rare packets of a stretch of capture: what the game sent around a
/// moment noted in game (a window opened, a run started), minus the traffic
/// that runs all the time.
///
///   A2_REPLAY_CAPTURE=… A2_BETWEEN=00:57:00-00:58:30 [A2_RARE=30] cargo test --release --test find_value_packets rare -- --ignored --nocapture
#[test]
#[ignore = "diagnostic"]
fn rare_packets_between() {
    let (Ok(capture), Ok(between)) = (std::env::var("A2_REPLAY_CAPTURE"), std::env::var("A2_BETWEEN")) else {
        eprintln!("A2_REPLAY_CAPTURE and A2_BETWEEN must both be set — skipping");
        return;
    };
    let rare: usize = std::env::var("A2_RARE").ok().and_then(|v| v.parse().ok()).unwrap_or(30);
    let (from, to) = between.split_once('-').expect("A2_BETWEEN as HH:MM:SS-HH:MM:SS");
    let packets = all_packets(&capture);
    let mut count: HashMap<[u8; 2], usize> = HashMap::new();
    for p in &packets {
        *count.entry(p.opcode).or_default() += 1;
    }
    for p in &packets {
        let at = clock(p.at_ms);
        if at.as_str() < from || at.as_str() > to || count[&p.opcode] > rare {
            continue;
        }
        println!("{}  {:02X} {:02X} ×{:<4} {:>5} B  {}", &at[..12], p.opcode[0], p.opcode[1], count[&p.opcode], p.body.len(), hex(&p.body, 90));
    }
}
