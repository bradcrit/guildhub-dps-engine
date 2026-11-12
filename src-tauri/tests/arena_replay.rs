//! Replays a capture of a 1v1 arena match through the stream processor and
//! checks the match `combat::arena` makes of it.
//!
//! The capture is the user's own of 2026-11-12 (`packets_20261112_194804.txt`):
//! map 61, against a Gladiator, lost the first round and won the next three.
//! Skips unless A2_ARENA_CAPTURE names it.

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
fn the_solo_arena_match_comes_out_three_to_one() {
    let Ok(path) = std::env::var("A2_ARENA_CAPTURE") else {
        eprintln!("A2_ARENA_CAPTURE unset; skipping");
        return;
    };
    let storage = Arc::new(DataStorage::new());
    let mut streams: HashMap<String, (StreamProcessor, PacketAccumulator)> = HashMap::new();
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
    }

    let matches = storage.arena_matches();
    for m in &matches {
        eprintln!("{:?} score {:?}", m.opponent, m.score());
        for r in &m.rounds {
            eprintln!("  {:?} dealt {} taken {}", r.winner, r.dealt, r.taken);
        }
    }
    let m = matches.first().expect("a match");
    assert_eq!(m.map, 61);
    assert_eq!(m.roster.as_ref().map(Vec::len), Some(2), "the roster (19 e3) was read");
    assert!(!m.is_team());
    let opponent = m.opponent.as_ref().expect("the opponent");
    assert_eq!(opponent.class, Some(JobClass::Gladiator));
    assert_eq!(m.score(), (3, 1));
    assert_eq!(m.rounds.iter().map(|r| r.winner).collect::<Vec<_>>(), [Some(Side::Them), Some(Side::Me), Some(Side::Me), Some(Side::Me)]);
    assert!(m.rounds.iter().all(|r| r.dealt > 0 && r.taken > 0));
    assert!(m.left_ms.is_some(), "left for map 1110");
}
