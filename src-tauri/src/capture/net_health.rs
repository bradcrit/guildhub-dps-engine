//! How the game's connection is doing, from the TCP headers alone.
//!
//! The game pings once every ten seconds, so a lag of a few seconds can fall
//! between two pings and a lag in progress shows nothing until it is over.
//! Its connection is busy, though: about 20 segments a second each way, in
//! town as in a fight (measured 2026-11-06). Three readings come from that,
//! and none of them touches a payload or sends anything:
//!
//! - **Round trip**: from a segment of ours to the server's acknowledgement of
//!   it. The server answers with its tick, about every 50 ms, so samples run
//!   from the real round trip to 50 ms above it. Our client ticks at the same
//!   rate and the two drift past each other in seconds, so even the lowest of
//!   a second or two swings (10 to 43 ms on a steady line): the lowest of
//!   five seconds is the reading, unless the last two sat more than a tick
//!   above it, which the drift can't do. The game's own ping runs a few ms
//!   above the reading (17 ms against 10), its processing counted in.
//! - **Resent share**: segments sent again, either way. TCP loses nothing for
//!   good: what the player feels as loss is the wait for the second copy. A
//!   hole in what was captured does not count, as the capture can cause that
//!   itself; a copy of what was already seen can't be its doing.
//! - **Stall**: how long our oldest segment has gone unacknowledged. The time
//!   since the server last spoke would do too, but this one stays silent in
//!   a quiet moment: nothing sent, nothing owed.
//!
//! Fed from the capture threads, ahead of the parser, as bare ACKs carry no
//! payload and never reach it.

use std::collections::VecDeque;

use parking_lot::Mutex;

/// Seconds whose lowest round trip is the reading.
const RTT_SECONDS: usize = 5;
/// The latest seconds, and how far above that reading their lowest must be to
/// be a slower connection rather than the two ticks' drift.
const SPIKE_SECONDS: usize = 2;
const SPIKE_ABOVE_MS: i64 = 60;
/// Seconds the resent share is taken over.
const RESENT_SECONDS: usize = 30;
/// Fewer segments than this say nothing about a share.
const MIN_SEGMENTS: u32 = 100;
/// No segment for this long: the game is gone. A blackout still has our own
/// resends in it, TCP's backoff keeping them under this.
const GONE_AFTER_MS: i64 = 15_000;
/// A stall counts from here. Reported in whole seconds, as a reading that
/// changes with every call would be sent with every call.
pub const STALL_FROM_MS: i64 = 1_000;
/// Longer than any round trip worth a reading.
const MAX_RTT_MS: i64 = 9_999;
/// Segments waiting for their acknowledgement.
const MAX_UNACKED: usize = 2_048;
/// A sequence number this far from the stream's is another stream (a
/// reconnect from the same ports), as in `tcp_order`.
const FAR: i64 = 4 * 1024 * 1024;

/// One TCP segment of a connection, as the capture saw it.
#[derive(Debug, Clone, Copy)]
pub struct Segment {
    pub from_server: bool,
    pub seq: u32,
    pub ack: u32,
    pub payload: u32,
    /// Capture time, for round trips: both ends on the capture's clock.
    pub at_ms: i64,
    /// Wall clock, for stalls. Npcap's clock drifts from it over days.
    pub now_ms: i64,
}

/// What `snapshot` reports.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NetReading {
    pub rtt_ms: Option<i64>,
    /// Share of segments sent again, in percent, to one decimal.
    pub resent_pct: f64,
    /// Whole seconds in ms, 0 under `STALL_FROM_MS`.
    pub stall_ms: i64,
}

impl NetReading {
    /// The round trip, unless the game's ping says it can't be the way to the
    /// server: a local tunnel (a ping reducer) acknowledges for it, in a
    /// millisecond or so. The game's ping is the older reading, so after a
    /// spike this may pass on nothing for a moment; it is then shown instead.
    pub fn rtt_beside(&self, game_ping_ms: Option<i32>) -> Option<i64> {
        let rtt = self.rtt_ms?;
        match game_ping_ms.map(i64::from) {
            Some(ping) if rtt * 5 < ping && ping - rtt >= 30 => None,
            _ => Some(rtt),
        }
    }
}

struct Unacked {
    end: u32,
    at_ms: i64,
    now_ms: i64,
    /// Sent again: its acknowledgement could be for either copy, so no round
    /// trip comes from it (Karn).
    resent: bool,
}

#[derive(Default)]
struct Second {
    at: i64,
    rtt_ms: Option<i64>,
    segments: u32,
    resent: u32,
}

#[derive(Default)]
struct Inner {
    connection: Option<(u16, String)>,
    seconds: VecDeque<Second>,
    unacked: VecDeque<Unacked>,
    /// Where each direction's sequence numbers have got to.
    client_high: Option<u32>,
    server_high: Option<u32>,
    last_seen_ms: i64,
}

#[derive(Default)]
pub struct NetHealth {
    inner: Mutex<Inner>,
}

/// Where `seq` lies from `from`, in bytes, wrapping as sequence numbers do.
fn distance(from: u32, seq: u32) -> i64 {
    seq.wrapping_sub(from) as i32 as i64
}

/// Whether `end` is past `high`, moving `high` along. `None`: a copy of what
/// was seen.
fn advance(high: &mut Option<u32>, end: u32) -> Option<bool> {
    let Some(at) = *high else {
        *high = Some(end);
        return Some(false);
    };
    let d = distance(at, end);
    if !(-FAR..=FAR).contains(&d) {
        *high = Some(end);
        return Some(true);
    }
    if d > 0 {
        *high = Some(end);
        return Some(false);
    }
    None
}

impl Inner {
    fn second(&mut self, at_ms: i64) -> &mut Second {
        let at = at_ms.div_euclid(1000);
        if self.seconds.back().is_none_or(|s| s.at < at) {
            self.seconds.push_back(Second { at, ..Second::default() });
            while self.seconds.len() > RESENT_SECONDS {
                self.seconds.pop_front();
            }
        }
        self.seconds.back_mut().expect("just pushed")
    }

    fn take_ack(&mut self, ack: u32, s: &Segment) {
        let mut last = None;
        while let Some(front) = self.unacked.front() {
            let d = distance(front.end, ack);
            if !(0..=FAR).contains(&d) {
                break;
            }
            last = self.unacked.pop_front();
            if let Some(first) = &last {
                let waited = s.now_ms - first.now_ms;
                if waited >= STALL_FROM_MS {
                    tracing::info!("Game connection stalled: {waited} ms without an acknowledgement");
                }
            }
        }
        // The newest it covers: the older ones may have waited for it.
        if let Some(acked) = last.filter(|u| !u.resent) {
            let rtt = s.at_ms - acked.at_ms;
            if (0..=MAX_RTT_MS).contains(&rtt) {
                let second = self.second(s.at_ms);
                second.rtt_ms = Some(second.rtt_ms.map_or(rtt, |low| low.min(rtt)));
            }
        }
    }

    fn take_client_data(&mut self, s: &Segment) {
        let end = s.seq.wrapping_add(s.payload);
        match advance(&mut self.client_high, end) {
            Some(new_stream) => {
                if new_stream {
                    self.unacked.clear();
                }
                self.unacked.push_back(Unacked { end, at_ms: s.at_ms, now_ms: s.now_ms, resent: false });
                while self.unacked.len() > MAX_UNACKED {
                    self.unacked.pop_front();
                }
            }
            None => {
                self.second(s.at_ms).resent += 1;
                for u in self.unacked.iter_mut().take_while(|u| distance(u.end, end) >= 0) {
                    u.resent = true;
                }
            }
        }
    }
}

impl NetHealth {
    /// Which connection is the game's: the one its data came on. Anything
    /// else from the same port (the server's TLS) is not measured.
    pub fn follow(&self, client_port: u16, device: &str) {
        let mut inner = self.inner.lock();
        if inner.connection.as_ref().is_none_or(|(port, name)| *port != client_port || name != device) {
            *inner = Inner { connection: Some((client_port, device.to_string())), ..Inner::default() };
        }
    }

    /// One segment of the locked port, on `device`.
    pub fn on_segment(&self, client_port: u16, device: &str, s: Segment) {
        let mut inner = self.inner.lock();
        if !inner.connection.as_ref().is_some_and(|(port, name)| *port == client_port && name == device) {
            return;
        }
        inner.last_seen_ms = s.now_ms;
        if s.from_server {
            inner.take_ack(s.ack, &s);
        }
        if s.payload == 0 {
            return;
        }
        inner.second(s.at_ms).segments += 1;
        if s.from_server {
            let end = s.seq.wrapping_add(s.payload);
            if advance(&mut inner.server_high, end).is_none() {
                inner.second(s.at_ms).resent += 1;
            }
        } else {
            inner.take_client_data(&s);
        }
    }

    /// The readings now, or `None` while no game connection is being seen.
    pub fn snapshot(&self, now_ms: i64) -> Option<NetReading> {
        let inner = self.inner.lock();
        inner.connection.as_ref()?;
        if inner.last_seen_ms == 0 || now_ms - inner.last_seen_ms >= GONE_AFTER_MS {
            return None;
        }
        let lowest = |seconds: usize| inner.seconds.iter().rev().take(seconds).filter_map(|s| s.rtt_ms).min();
        let rtt_ms = match (lowest(RTT_SECONDS), lowest(SPIKE_SECONDS)) {
            (Some(base), Some(latest)) if latest > base + SPIKE_ABOVE_MS => Some(latest),
            (base, _) => base,
        };
        let segments: u32 = inner.seconds.iter().map(|s| s.segments).sum();
        let resent: u32 = inner.seconds.iter().map(|s| s.resent).sum();
        let resent_pct = if segments < MIN_SEGMENTS {
            0.0
        } else {
            (f64::from(resent) * 1000.0 / f64::from(segments)).round() / 10.0
        };
        let waited = inner.unacked.front().map_or(0, |u| now_ms - u.now_ms);
        let stall_ms = if waited >= STALL_FROM_MS { waited / 1000 * 1000 } else { 0 };
        Some(NetReading { rtt_ms, resent_pct, stall_ms })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PORT: u16 = 51000;
    const DEVICE: &str = "Ethernet";
    const T0: i64 = 1_800_000_000_000;

    fn seg(from_server: bool, seq: u32, ack: u32, payload: u32, ms: i64) -> Segment {
        Segment { from_server, seq, ack, payload, at_ms: T0 + ms, now_ms: T0 + ms }
    }

    fn health() -> NetHealth {
        let h = NetHealth::default();
        h.follow(PORT, DEVICE);
        h
    }

    fn feed(h: &NetHealth, s: Segment) {
        h.on_segment(PORT, DEVICE, s);
    }

    #[test]
    fn the_round_trip_is_the_lowest_of_the_last_seconds() {
        let h = health();
        // Acknowledged with the server's tick: 60 ms, 12 ms, 35 ms.
        feed(&h, seg(false, 100, 500, 10, 0));
        feed(&h, seg(true, 500, 110, 20, 60));
        feed(&h, seg(false, 110, 520, 10, 100));
        feed(&h, seg(true, 520, 120, 20, 112));
        feed(&h, seg(false, 120, 540, 10, 200));
        feed(&h, seg(true, 540, 130, 20, 235));
        assert_eq!(h.snapshot(T0 + 300).unwrap().rtt_ms, Some(12));
    }

    #[test]
    fn a_round_trip_far_under_the_games_ping_is_a_tunnels() {
        let reading = NetReading { rtt_ms: Some(1), resent_pct: 0.0, stall_ms: 0 };
        assert_eq!(reading.rtt_beside(Some(40)), None);
        let reading = NetReading { rtt_ms: Some(10), ..reading };
        assert_eq!(reading.rtt_beside(Some(17)), Some(10), "the game's runs a little higher");
        assert_eq!(reading.rtt_beside(Some(40)), Some(10), "and jumps now and then");
        assert_eq!(reading.rtt_beside(None), Some(10));
    }

    #[test]
    fn a_bare_ack_gives_a_round_trip_too() {
        let h = health();
        feed(&h, seg(false, 100, 500, 10, 0));
        feed(&h, seg(true, 500, 110, 0, 14));
        assert_eq!(h.snapshot(T0 + 20).unwrap().rtt_ms, Some(14));
    }

    #[test]
    fn a_slower_connection_shows_once_the_old_seconds_have_gone() {
        let h = health();
        let mut seq = 100;
        for second in 0..5 {
            let ms = second * 1000;
            let rtt = if second < 2 { 15 } else { 400 };
            feed(&h, seg(false, seq, 500, 10, ms));
            seq += 10;
            feed(&h, seg(true, 500, seq, 0, ms + rtt));
        }
        assert_eq!(h.snapshot(T0 + 4500).unwrap().rtt_ms, Some(400));
    }

    #[test]
    fn the_ticks_drifting_apart_is_not_a_slower_connection() {
        let h = health();
        let mut seq = 100;
        for (second, rtt) in [10, 12, 43, 40, 35].into_iter().enumerate() {
            let ms = second as i64 * 1000;
            feed(&h, seg(false, seq, 500, 10, ms));
            seq += 10;
            feed(&h, seg(true, 500, seq, 0, ms + rtt));
        }
        assert_eq!(h.snapshot(T0 + 4500).unwrap().rtt_ms, Some(10));
    }

    #[test]
    fn segments_sent_again_are_counted_either_way() {
        let h = health();
        let (mut ours, mut theirs) = (100u32, 500u32);
        for i in 0..100 {
            feed(&h, seg(false, ours, theirs, 10, i * 20));
            ours += 10;
            feed(&h, seg(true, theirs, ours, 10, i * 20 + 5));
            theirs += 10;
        }
        assert_eq!(h.snapshot(T0 + 2100).unwrap().resent_pct, 0.0);
        feed(&h, seg(false, ours - 10, theirs, 10, 2100));
        feed(&h, seg(true, theirs - 10, ours, 10, 2105));
        // 2 of 202.
        assert_eq!(h.snapshot(T0 + 2110).unwrap().resent_pct, 1.0);
    }

    #[test]
    fn a_hole_is_not_a_resend() {
        let h = health();
        let mut theirs = 500u32;
        for i in 0..120 {
            // Every other segment never reached the capture.
            feed(&h, seg(true, theirs, 100, 10, i * 10));
            theirs += 20;
        }
        assert_eq!(h.snapshot(T0 + 1300).unwrap().resent_pct, 0.0);
    }

    #[test]
    fn too_few_segments_say_nothing() {
        let h = health();
        feed(&h, seg(false, 100, 500, 10, 0));
        feed(&h, seg(false, 100, 500, 10, 200));
        assert_eq!(h.snapshot(T0 + 300).unwrap().resent_pct, 0.0);
    }

    #[test]
    fn what_was_sent_again_gives_no_round_trip() {
        let h = health();
        feed(&h, seg(false, 100, 500, 10, 0));
        feed(&h, seg(false, 100, 500, 10, 300));
        feed(&h, seg(true, 500, 110, 0, 310));
        assert_eq!(h.snapshot(T0 + 320).unwrap().rtt_ms, None);
    }

    #[test]
    fn a_stall_counts_from_the_oldest_segment_owed() {
        let h = health();
        feed(&h, seg(false, 100, 500, 10, 0));
        feed(&h, seg(false, 110, 500, 10, 50));
        assert_eq!(h.snapshot(T0 + 900).unwrap().stall_ms, 0);
        assert_eq!(h.snapshot(T0 + 1000).unwrap().stall_ms, 1000);
        assert_eq!(h.snapshot(T0 + 3400).unwrap().stall_ms, 3000);
        feed(&h, seg(true, 500, 120, 0, 3500));
        assert_eq!(h.snapshot(T0 + 3500).unwrap().stall_ms, 0);
    }

    #[test]
    fn nothing_sent_is_no_stall() {
        let h = health();
        feed(&h, seg(false, 100, 500, 10, 0));
        feed(&h, seg(true, 500, 110, 0, 10));
        assert_eq!(h.snapshot(T0 + 8000).unwrap().stall_ms, 0);
    }

    #[test]
    fn sequence_numbers_wrap() {
        let h = health();
        feed(&h, seg(false, u32::MAX - 4, 500, 10, 0));
        feed(&h, seg(true, 500, 5, 0, 12));
        assert_eq!(h.snapshot(T0 + 20).unwrap().rtt_ms, Some(12));
        assert_eq!(h.snapshot(T0 + 20).unwrap().stall_ms, 0);
    }

    #[test]
    fn another_connection_is_not_measured() {
        let h = health();
        h.on_segment(PORT + 1, DEVICE, seg(false, 100, 500, 10, 0));
        h.on_segment(PORT, "Other", seg(false, 100, 500, 10, 0));
        assert_eq!(h.snapshot(T0 + 10), None);
    }

    #[test]
    fn following_another_connection_starts_over() {
        let h = health();
        feed(&h, seg(false, 100, 500, 10, 0));
        h.follow(PORT + 1, DEVICE);
        assert_eq!(h.snapshot(T0 + 5000), None);
    }

    #[test]
    fn a_game_gone_quiet_reads_nothing() {
        let h = health();
        feed(&h, seg(false, 100, 500, 10, 0));
        assert!(h.snapshot(T0 + 14_000).is_some());
        assert_eq!(h.snapshot(T0 + 15_000), None);
    }

    #[test]
    fn a_stream_from_far_away_starts_over() {
        let h = health();
        feed(&h, seg(false, 100, 500, 10, 0));
        feed(&h, seg(false, 2_000_000_000, 500, 10, 100));
        feed(&h, seg(true, 500, 2_000_000_010, 0, 120));
        let reading = h.snapshot(T0 + 130).unwrap();
        assert_eq!(reading.rtt_ms, Some(20));
        assert_eq!(reading.stall_ms, 0);
    }
}
