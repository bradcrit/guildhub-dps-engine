//! The Commendations window after a dungeon, where the party votes for one of
//! its own. Read 2026-11-09 on a five-player Fire Temple run (instance 600022),
//! its times matched against what the player saw:
//!
//! ```text
//! 00 44 <n u8> …      who can be voted for: the party without you. Came in the
//!                     same tick as the last boss's death: the window opens.
//! 02 44 00 00         29 ms after the client sent its vote: the window closes.
//! 03 44 …, 04 44 …    the round's result, 18.5 s after the 00 44 (with everyone
//!                     having voted): nobody's window is left open.
//! ```
//!
//! Seen once each in that run, and in none of four captures in the open world.
//! No run without a vote, or with a timed-out round, was captured.

use super::stream_processor::read_varint;

const OPEN: [u8; 2] = [0x00, 0x44];
const CLOSING: [[u8; 2]; 3] = [[0x02, 0x44], [0x03, 0x44], [0x04, 0x44]];
/// The smallest entry of the `00 44` list, with a one-letter name.
const MIN_ENTRY: usize = 18;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Commendation {
    Opened,
    Closed,
}

/// A whole frame (its length varint, type, body): what it says of the window, if anything.
pub fn read(frame: &[u8]) -> Option<Commendation> {
    let at = usize::try_from(read_varint(frame, 0).length).ok().filter(|&n| n > 0)?;
    let kind: [u8; 2] = frame.get(at..at + 2)?.try_into().ok()?;
    let body = &frame[at + 2..];
    if kind == OPEN {
        let n = usize::from(*body.first()?);
        return (n > 0 && body.len() >= 1 + n * MIN_ENTRY).then_some(Commendation::Opened);
    }
    CLOSING.contains(&kind).then_some(Commendation::Closed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// The frame of 2026-11-09, the four names swapped for made-up ones of the same length.
    const LIST: &str = "6D0044040016051C620300000016050\
        4416E6E61062D0000000400230568590300000023050942657274686F6C6474082D000000\
        001E05381B010000002A05054361726C61052D000000\
        000E09BEEF030000000E0907446F6D696E696B092D0000003B2F03C0A1010000";

    #[test]
    fn the_party_list_opens_the_window() {
        assert_eq!(read(&hex(LIST)), Some(Commendation::Opened));
    }

    #[test]
    fn the_vote_and_the_result_close_it() {
        assert_eq!(read(&hex("0802440000")), Some(Commendation::Closed));
        assert_eq!(read(&hex("0E03440201235968590300")), Some(Commendation::Closed));
        assert_eq!(read(&hex("0E04440501000000850000")), Some(Commendation::Closed));
    }

    #[test]
    fn a_frame_too_short_for_its_list_opens_nothing() {
        assert_eq!(read(&hex("080044040000")), None);
        assert_eq!(read(&hex("060044")), None);
        assert_eq!(read(&hex("08004400001122")), None);
    }

    #[test]
    fn other_frames_say_nothing() {
        assert_eq!(read(&hex("0E0036719301")), None);
        assert_eq!(read(&hex("0E0544719301")), None);
        assert_eq!(read(&[]), None);
    }
}
