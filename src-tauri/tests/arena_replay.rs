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
//! - `packets_20261112_225001.txt`: a 5v5 on map 60, your team (2) wiped in
//!   one round without a kill.
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
    if m.is_team() {
        for p in &m.players {
            eprintln!("  team {} {:14} {:?} IL {:?} id {} dealt {} taken {} deaths {}", p.team, p.name, p.class, p.item_level, p.id, p.dealt, p.taken, p.deaths);
        }
        assert_eq!(m.players.len(), 10);
        assert_eq!(m.my_team, Some(2));
        assert!(m.players.iter().all(|p| p.id > 0), "everyone's entity known");
        assert_eq!(m.rounds.iter().map(|r| r.winner).collect::<Vec<_>>(), [Some(Side::Them)]);
        let deaths = |team| m.players.iter().filter(|p| p.team == team).map(|p| p.deaths).sum::<u32>();
        assert_eq!((deaths(1), deaths(2)), (0, 5));
        assert!(m.players.iter().all(|p| p.dealt > 0 || p.taken > 0));
        assert!(storage.get_party_members().is_empty(), "the arena's party went with the match (1f 92)");
        return;
    }
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
    // The game's own result (`28 e3`), where the capture has the match's end.
    let (won, lost) = m.score();
    if won >= 3 || lost >= 3 {
        assert_eq!(m.result, Some(if won > lost { Side::Me } else { Side::Them }));
    }
}

/// Every packet of a capture through the stream processor.
fn replay(path: &str) -> Arc<DataStorage> {
    let storage = Arc::new(DataStorage::new());
    let mut streams: HashMap<String, (StreamProcessor, PacketAccumulator)> = HashMap::new();
    for cap in share::read_capture(std::path::Path::new(path)).expect("capture") {
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
    storage
}

/// The user's 10v10 of 2026-11-13 (A2_BG_CAPTURE, `packets_20261113_111906.txt`;
/// skips when unset): map 62 "Arena of Tactics", Bradcrit on red (team 2),
/// red won 85:50. The game's end screen, player by player.
#[test]
fn a_battleground_reads_as_the_game_s_end_screen() {
    let Ok(path) = std::env::var("A2_BG_CAPTURE") else {
        eprintln!("A2_BG_CAPTURE unset; skipping");
        return;
    };
    let storage = replay(&path);
    let matches = storage.arena_matches();
    let m = matches.iter().find(|m| m.battleground).expect("a battleground");
    for p in &m.players {
        eprintln!("  team {} {:14} {:?} IL {:?} id {} dealt {} taken {} {}/{}/{}", p.team, p.name, p.class, p.item_level, p.id, p.dealt, p.taken, p.kills, p.deaths, p.assists);
    }
    assert_eq!(m.map, 62);
    assert_eq!(m.players.len(), 20);
    assert_eq!(m.my_team, Some(2));
    assert!(m.rounds.is_empty());
    let end_screen: [(&str, u32, u32, u32); 20] = [
        ("StyZZy", 14, 8, 15),
        ("xrayder", 3, 10, 5),
        ("Paula", 5, 5, 20),
        ("Moxxy", 4, 7, 18),
        ("Jankes", 3, 9, 14),
        ("IndecisoITA", 6, 11, 4),
        ("XiaFeng", 2, 10, 14),
        ("Hope", 1, 11, 7),
        ("Scarx", 10, 7, 15),
        ("Benihimê", 2, 7, 11),
        ("lucky17", 4, 4, 25),
        ("Bradcrit", 4, 8, 22),
        ("GogoYubari", 3, 5, 23),
        ("Shû", 18, 5, 27),
        ("NikoGoesWild", 8, 4, 26),
        ("Roro", 5, 5, 19),
        ("lokie", 5, 7, 16),
        ("RalVln", 10, 5, 21),
        ("Seia", 5, 5, 20),
        ("Razet", 22, 2, 28),
    ];
    for (name, k, d, a) in end_screen {
        let p = m.players.iter().find(|p| p.name == name).unwrap_or_else(|| panic!("{name} on the roster"));
        assert_eq!((p.kills, p.deaths, p.assists), (k, d, a), "{name}");
    }
    // A team's score is the other team's deaths.
    let deaths = |team| m.players.iter().filter(|p| p.team == team).map(|p| p.deaths).sum::<u32>();
    assert_eq!((deaths(1), deaths(2)), (85, 50));
    assert_eq!(m.result, Some(Side::Me));
    // 11:32:04 local (UTC+1): ten minutes after the start at 11:22:04.
    assert_eq!(m.ends_ms, Some(1_794_565_924_095));
    assert!(m.players.iter().any(|p| p.dealt > 0), "damage between the teams");
    assert!(m.left_ms.is_some(), "left the battleground");
}
