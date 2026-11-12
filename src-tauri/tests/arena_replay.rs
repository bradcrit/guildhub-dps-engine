//! Replays captures of 1v1 arena matches through the stream processor and
//! checks the match `combat::arena` makes of them.
//!
//! The user's own captures of 2026-11-12 (A2_ARENA_CAPTURE, one path; skips
//! when unset):
//! - `packets_20261112_194804.txt`: map 61 against StayHype, a Gladiator —
//!   lost the first round, won the next three (3:1). The roster came 50 ms
//!   before the self record.
//! - `packets_20261112_203716.txt`: map 60 against Freydis, a Gladiator — 3:0.
//! - `packets_20261112_224035.txt`: map 61 against kalandra, a Sorcerer —
//!   lost, lost, won, won, lost (2:3). First to three wins the match.
//!
//! Either way the opponent is named by the roster before the first round.

use std::collections::HashMap;
use std::sync::Arc;

use a2tools_dps_meter_lib::capture::packet_accumulator::PacketAccumulator;
use a2tools_dps_meter_lib::capture::stream_processor::StreamProcessor;
use a2tools_dps_meter_lib::combat::arena::Side;
use a2tools_dps_meter_lib::combat::data_storage::DataStorage;
use a2tools_dps_meter_lib::entity::job_class::JobClass;
use a2tools_dps_meter_lib::i18n::lookup::{NpcLookup, SkillLookup};
use a2tools_dps_meter_lib::share;

#[test]
fn a_solo_arena_match_reads_as_the_user_saw_it() {
    let Ok(path) = std::env::var("A2_ARENA_CAPTURE") else {
        eprintln!("A2_ARENA_CAPTURE unset; skipping");
        return;
    };
    let storage = Arc::new(DataStorage::new());
    let mut streams: HashMap<String, (StreamProcessor, PacketAccumulator)> = HashMap::new();
    // Was the opponent known (with a server) before the first round began?
    let mut named_before_rounds: Option<bool> = None;
    for cap in share::read_capture(std::path::Path::new(&path)).expect("capture") {
        let (processor, acc) = streams.entry(cap.stream.clone()).or_insert_with(|| {
            let p = StreamProcessor::new(storage.clone(), Arc::new(SkillLookup::new()), Arc::new(NpcLookup::new()));
            (p, PacketAccumulator::new())
        });
        acc.append(&cap.bytes);
        let consumed = processor.consume_stream(acc.snapshot());
        if consumed > 0 {
            acc.discard_bytes(consumed);
        }
        if named_before_rounds.is_none() {
            if let Some(m) = storage.arena_matches().first() {
                if m.opponent.as_ref().is_some_and(|o| o.server.is_some()) || !m.rounds.is_empty() {
                    named_before_rounds = Some(m.rounds.is_empty());
                }
            }
        }
    }

    let matches = storage.arena_matches();
    for m in &matches {
        eprintln!("map {} {:?} score {:?}", m.map, m.opponent, m.score());
        for r in &m.rounds {
            eprintln!("  {:?} dealt {} taken {}", r.winner, r.dealt, r.taken);
        }
    }
    let m = matches.first().expect("a match");
    assert_eq!(m.roster.as_ref().map(Vec::len), Some(2), "the roster (19 e3) was read");
    assert!(!m.is_team());
    assert_eq!(named_before_rounds, Some(true), "the roster named the opponent before the first round");
    let opponent = m.opponent.as_ref().expect("the opponent");
    assert!(opponent.id > 0 && opponent.item_level.is_some(), "their spawn gave the entity and item level");
    let (me, them) = (Some(Side::Me), Some(Side::Them));
    let (class, rounds) = match opponent.name.as_str() {
        "StayHype" => (JobClass::Gladiator, vec![them, me, me, me]),
        "Freydis" => (JobClass::Gladiator, vec![me, me, me]),
        "kalandra" => (JobClass::Sorcerer, vec![them, them, me, me, them]),
        other => panic!("an opponent of no capture: {other}"),
    };
    assert_eq!(opponent.class, Some(class));
    assert_eq!(m.rounds.iter().map(|r| r.winner).collect::<Vec<_>>(), rounds);
    assert!(m.rounds.iter().all(|r| r.dealt > 0 && r.taken > 0));
    assert!(m.left_ms.is_some(), "left for map 1110");
}
