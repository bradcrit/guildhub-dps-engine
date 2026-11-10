//! `guildhub-dps-engine` — the A2Tools DPS Meter without its windows, for
//! GuildHub Companion.
//!
//! This file is part of a fork of the A2Tools DPS Meter by taengu
//! (https://github.com/taengu/A2Tools-DPS-Meter), licensed GPL-3.0 like the
//! rest of this repository. The parser, capture and combat code is upstream's,
//! unchanged; this binary only wires it up without Tauri windows and hands the
//! results to another process over a local HTTP connection.
//!
//! Built with the default features: the core records field boss kills only
//! with `online`. What that feature adds to the meter (uploads, Discord
//! status, the updater …) is started by `app.rs`, which this binary doesn't
//! use — the engine itself talks to nothing but its parent.
//!
//! ```text
//! guildhub-dps-engine --data-dir <dir> --resources <dir> [--language en]
//! ```
//!
//! - `GH_ENGINE_TOKEN` (environment, required): the shared secret every
//!   request has to carry, as `Authorization: Bearer <token>` or `?token=`.
//! - `GH_ENGINE_SAVE_PACKETS=1` (environment, optional): the raw game traffic
//!   is written to `<data-dir>/packets/packets_*.txt`, as the meter's "save raw
//!   packets" setting does — captures to find new records in
//!   (`tests/find_value_packets.rs`). The parent passes its own environment on.
//! - The first line on stdout is `{"event":"ready","port":N,"version":"..."}`;
//!   the server listens on `127.0.0.1:N` only.
//! - When stdin closes (the parent process is gone) the engine saves the open
//!   fights and exits.
//!
//! Routes:
//!
//! | Route                  | Answer                                          |
//! |------------------------|-------------------------------------------------|
//! | `GET  /v1/status`      | version, Npcap, capture, game window            |
//! | `GET  /v1/dps`         | the meter's current view (`DpsData`, plus `targetDead`, `targetStagger`, in a language other than English or German `targetNameEn`) |
//! | `GET  /v1/self`        | your character, its Odyle energy, entry counts and currencies (`lists`), the ping |
//! | `GET  /v1/events`      | SSE, see below                                  |
//! | `GET  /v1/fights`      | saved fights (`FightSummary[]`)                 |
//! | `GET  /v1/fights/<id>` | one saved fight (`FightRecord`)                 |
//! | `POST /v1/reset`       | save the open fights, then clear the meter      |
//! | `POST /v1/config?mode=<id>&window=<ms>` | which targets the meter follows (`TargetSelectionMode` ids) and the all-targets window |
//! | `POST /v1/character?name=<name>` | the character played, as the parent read it off the game's screen just now |
//!
//! Events: `dps` (the view, each `map` entry with its player's home `server`,
//! null while not known) and `self` (as `/v1/self`) on every change, each
//! sent once on connecting (`self` also carries the game's `ping` in ms);
//! `net` (`{"rtt","resentPct","stallMs","serverLagMs"}`, `rtt` on the scale of
//! the game's ping, null without a game connection) on
//! every change, at most each second, and once on connecting: the connection
//! read from the TCP headers (`capture::net_health`);
//! `fight` (`{"id","killed"}`) when a fight is written to
//! history; `bossKill` (`{"npc","at","server"}`) when a field boss dies in the
//! open world; `bossMap` (`{"server","map","seenAt","bosses":[{"npc","up","at"}]}`)
//! when the player opened a map of their own server and it says something new
//! (or again after 5 minutes): each field boss on it, up since `at` or back at
//! `at` (unix ms, 0 when the game gives none); `bossHp`
//! (`{"server","at","bosses":[{"npc","hp","reset"}]}`, every 3 s at most) with
//! the field bosses in the open world whose HP changed, and the ones still
//! fought once a minute — `hp` whole percent left, `reset` when it came back
//! up. `server` is the game's server id (null for a kill before the game
//! named it). `pvp` (`{"players":[{"id","name","class","itemLevel","rank","seenAt",
//! "attackedAt","dealt","taken"}],"abyss","death"}`) on every change and once on
//! connecting: the players of the other faction in sight, attacking you first
//! (latest attack first), then the one seen last first — `seenAt`/`attackedAt`
//! unix ms, `attackedAt` null for one who hasn't hit you, `class` and
//! `itemLevel` null while not known, `rank` the Abyss rank (19 Trainee, 18
//! Ninth-Class … 10 First-Class Soldier; null: not known), `dealt`/`taken` the damage you dealt them
//! and they dealt you since they came into sight; `abyss` while the map you
//! are on is the Abyss; `death` the recap of your last death by players' hands
//! (`{"at","killer","skill","abyss","attackers":[{"name","class","rank","damage","hits","skill"}]}`,
//! most damage first, `skill` a name, "" when the game's isn't known) or null;
//! `kills` your last 50 kills and assists of enemy players, oldest first
//! (`{"at","name","class","itemLevel","rank","killingBlow","damage","hits","skill",
//! "total","helpers","abyss"}`: the last hit yours, your damage, hits and top
//! skill, everyone's damage and how many others hit them, in the 15 s before);
//! `abyssPoints` the Abyss Points you gained lately (`{"at","amount"}`, oldest
//! first: each rise of the total the game sends, from any source — not the
//! first total after a start or a character switch).
//! `self` also has your `itemLevel` and `combatPower` (a party roster's, else
//! your last self record's; null before either), and `map`: the map id of the
//! last zone load (null before one).
//!
//! Pets (the game's own records, read as A2Tools' Pet Soul Tracker reads
//! them; a pet collection is shared by a server's characters): `petList`
//! (`{"server","at","levels":[{"pet","level"}],"points":[{"pet","points"}]}`)
//! once per world entry — each pet's level and its souls toward the next —
//! passed on PET_LIST_SETTLE_MS after it was read, so that a server switch
//! has named the new server by then; `petSouls`
//! (`{"server","at","souls":[{"pet","amount"}]}`) as souls are picked up,
//! one kill can feed several pets; `petLevel` (`{"server","at","pet","level"}`)
//! when a pet reaches a level (the game resets its souls to 0 then, and
//! doesn't send this for every step). `pet` is the game's pet id (1001 …).
//! `petShop` (`{"server","at","tab","refreshAt","offers":[{"item","count",
//! "tier","price","original"}]}`) each time a tab of the Black Cloud
//! Merchant's shop is opened (`91 56`; the server sends it only then, not at
//! login), passed on at once — `server` null while not known yet: `tab` from its listing ids (21 Special, 22 Pets, 23 Skins),
//! `refreshAt` when the shop changes its wares next (unix ms, the full hour;
//! null: not read), `item` the game's item id (a pet's is its soul item's
//! + 100000), `tier` the discount (0 −10 %, 1 −20 %, 2 −30 %, 3 −50 %),
//! `price` and `original` in Kinah.
//! `npcs` (`{"npcs":[{"npc","count"}],"target"}`) on every change and once on
//! connecting: the living mobs in sight by NPC code, from their spawn until
//! they die, despawn or a zone load — what the pet tracker farms nearby.
//! `target` is the NPC code of the mob you fight (the one in sight you hit
//! directly last, within TARGET_HIT_MS), else null. Each npc has `metres`, how
//! far the closest of them is from you, while your spot is known
//! (`capture::positions`, `DataStorage::self_estimate`), else null; `posAge`
//! (ms) is how old the spot the game told is (null: none, or more than
//! SELF_POS_MAX_AGE_MS ago), `pos` (`{"x","y"}`) your spot on the ground in
//! metres, to the nearest 2 (null: unknown) — the pet tracker tells from it
//! that you left an area. `posGuessed`: `pos` is moved off the told spot
//! since, by the mobs and objects that left sight (`DataStorage::self_estimate`)
//! — tens of metres off; kept while it keeps moving, past the told spot's
//! SELF_POS_MAX_AGE_MS (`posAge` null then). Gathering (opening) an object
//! tells your spot: its own. A guess is also the middle of where everything
//! in range and what came and went lately puts you (`DataStorage::self_fix`).
//! `posAccuracy`: how far off `pos` may be (m, to 5): a guess's own, a spot
//! told's when told (3 m, to 5) — the app grows it by running since `posAge`. Each npc has `spots` too, `[x, y]` in metres (to the
//! nearest MOB_SPOT_STEP_M) of each of them whose spot the game told; `kills`
//! (`[{"npc","x","y","at"}]`) are the mobs in sight that died in the last
//! MOB_KILLS_SHOWN_MS, where (same metres) and when (unix ms) — the pet
//! tracker's respawn timers.
//! `cubes` (`{"cubes":[{"metres","above","age","opening"}],"opened"}`) on
//! every change and once on connecting: the Hidden Cubes in range
//! (`capture::objects`), closest first, from when the game sends one until it
//! goes out of range, is opened or a zone load. `metres` is how far it is on
//! the ground, `above` how far above you (below: negative), both whole
//! metres, while your spot is at most CUBE_POS_MAX_AGE_MS old, else null;
//! `age` how long it has been there in seconds — 0 under a minute, then whole
//! minutes — null where the game didn't say; `opening` "you" or "other" once
//! someone started to open it, else null. `opened`: the cubes you opened (a
//! key used) since the engine started.
//! `skills` (`{"cooldowns","buffs","target"}`, every 100 ms at most, on a
//! change, and once on connecting) for the cooldown and cast bars
//! (`capture::skill_casts`, `combat::skill_timers`): `cooldowns` your skills
//! on cooldown, the soonest back first (`{"skill","name","icon","endsAt",
//! "totalMs"}`, `skill` without its specialization digits); `buffs` the
//! buffs and debuffs on you (`{"abnormal","skill","name","icon","endsAt",
//! "totalMs","stacks","mine","hostile"}`, `endsAt`/`totalMs` null for one
//! without a timer, `mine` cast by you, `hostile` a debuff — as the game data
//! files it, or for an abnormal it doesn't list, cast by a monster or a player
//! of the other faction), the soonest to end first; `target` what the
//! monster you fight is up to (`{"entity","cast","deadline"}`, null while
//! you fight none): `cast` the skill it is casting
//! (`{"skill","name","startedAt","castMs"}`, `castMs` learned from its earlier
//! casts, null while not known — the cast lands about then) or null,
//! `deadline` its time limit (unix ms) or null. Every time is unix ms.
//! `icon` is a file name on NC's icon CDN (null: none known) — an abnormal
//! without one of its own gets its skill's, as it gets its skill's name.
//! `resources` (`{"hp","mp","stamina","flight"}`, every 100 ms at most, on a
//! change, and once on connecting) for the resource bars
//! (`capture::resources`): each `{"current","max"}` of your character, null
//! until its maximum is known (the login sends it; a meter started
//! mid-session hears it at the next login — HP in a party at once, from its
//! `1b 92`).
//! `party` (`{"members":[{"id","name","class","server","level","combatPower","itemLevel","slot","me","near","hp","mp",
//! "dead","debuffs"}]}`, every 100 ms at most, on a change, and once on connecting)
//! for the party frames: the roster's members in slot order (empty outside a
//! party), `near` while they are in sight (you always; spawned and not gone
//! since, as the game fades a member far off in its own frames), `level` as the roster states it (null: 0 there), `id` their entity (null while not seen), `hp`/`mp` each
//! `{"current","max"}` or null (`1b 92`, about once a second while it
//! changes; yours from your resources), `dead` at 0 HP; `debuffs` the debuffs
//! on them (`{"abnormal","name","icon","endsAt","totalMs","stacks",
//! "cleansable"}`, as `skills`' buffs), the ones a party member can take off
//! first — `cleansable` learnt from other players' skills taking them off
//! (`<data-dir>/cleansable.json`). `listed`: whether the meter lists the
//! party before a fight — false after ↻ until the next roster, and ten minutes
//! after the last roster.
//! `loot` (`{"at","source","dungeon","runAt","items":[{"item","count"}]}`)
//! as you loot (`capture::loot`): `source` "cube" — a cube's whole reward
//! list — or "item", one item that went into your inventory (a boss's soul,
//! Kinah from a mob). `item` is the game's item id. `dungeon` the instance
//! you are in (0: the open world), `runAt` when it was entered (unix ms,
//! the fights of that run carry it as `runAt`; 0 when not seen). Your own
//! loot only: the game says nothing of anyone else's.
//! `gather` (`{"at","outcome","item","count","bonus","node","map"}`) as a
//! round of your own gathering ends (`capture::gathering`): `outcome`
//! "plain", "better" (the material a grade up) or "failed" (`item` then the
//! material it would have been, `count` 0); `count` with the `bonus` on top
//! in it; `node` the node's object code and `map` the map (null: not known).
//! `commendation` (`{"open"}`) at once as the Commendations window after a
//! dungeon opens (true) or closes (false: your vote, or the round's result —
//! also long after you voted; `capture::commendation`).

use std::collections::{HashMap, HashSet};
use std::io::BufRead;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, watch};

use a2tools_dps_meter_lib::capture::boss_map::BossMap;
use a2tools_dps_meter_lib::capture::captured_payload::CapturedPayload;
use a2tools_dps_meter_lib::capture::field_bosses::npc_of_map_id;
use a2tools_dps_meter_lib::capture::game_lists::CURRENCIES;
use a2tools_dps_meter_lib::capture::commendation::{self, Commendation};
use a2tools_dps_meter_lib::capture::gathering::{self, Gathered};
use a2tools_dps_meter_lib::capture::loot::{Loot, LootReader};
use a2tools_dps_meter_lib::capture::combat_port_detector::CombatPortDetector;
use a2tools_dps_meter_lib::capture::pcap_capturer::PcapCapturer;
use a2tools_dps_meter_lib::combat::capture_dispatcher::CaptureDispatcher;
use a2tools_dps_meter_lib::combat::data_storage::DataStorage;
use a2tools_dps_meter_lib::combat::dps_calculator::DpsCalculator;
use a2tools_dps_meter_lib::combat::kda::Kind as KdaKind;
use a2tools_dps_meter_lib::combat::ping_tracker::PingTracker;
use a2tools_dps_meter_lib::combat::data_storage::BossEventKind;
use a2tools_dps_meter_lib::entity::fight_record::FightRecord;
use a2tools_dps_meter_lib::entity::job_class::JobClass;
use a2tools_dps_meter_lib::history::fight_history::{FightHistoryManager, is_plain_name};
use a2tools_dps_meter_lib::i18n::lookup::{self, NpcLookup, SkillLookup};
use a2tools_dps_meter_lib::platform;
use a2tools_dps_meter_lib::capture::frame_tap;
use a2tools_dps_meter_lib::capture::stream_processor::read_varint;
use a2tools_dps_meter_lib::plugins::decoder::{Decoder, DecoderSpec};

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// How often the meter's view is recalculated (upstream's default rate).
const DPS_RATE: Duration = Duration::from_millis(200);
/// How often open boss fights are written to history (as upstream does).
const SAVE_EVERY: Duration = Duration::from_secs(30);
/// How often your character, Odyle energy and field boss kills are looked at.
const SELF_RATE: Duration = Duration::from_secs(1);
/// `npcs`: the mobs' spots to the nearest this many metres.
const MOB_SPOT_STEP_M: f32 = 5.0;
/// `npcs`: the kills of mobs in sight this recent (ms) — each comes in several events, the Companion takes it once.
const MOB_KILLS_SHOWN_MS: i64 = 60_000;
/// The mob you hit last counts as the one you fight for this long (`npcs`).
const TARGET_HIT_MS: i64 = 15_000;
/// Distances (`npcs`) from your spot while it is at most this old.
const SELF_POS_MAX_AGE_MS: i64 = 60_000;
/// A spot the game told is this exact (m): `posAccuracy` while not guessed.
const TOLD_ACCURACY_M: f32 = 3.0;
/// Distances (`cubes`) from your spot while it is at most this old: off a
/// fight the game tells your spot only every few seconds.
const CUBE_POS_MAX_AGE_MS: i64 = 5_000;
/// How far each NPC code leaves sight, learnt for guessing your spot
/// (`DataStorage::self_estimate`), kept for the next session in
/// `<data-dir>/sight_radii.json` (`{"<npc code>":[cm, …]}`): a session
/// starts knowing them. Written at most this often, when it learnt more.
const SIGHT_RADII_FILE: &str = "sight_radii.json";
const SIGHT_RADII_SAVE_EVERY: Duration = Duration::from_secs(60);
/// How often field boss HP is looked at: only bosses whose percent moved
/// go out, so a fight shows live on the radar without idle traffic.
const BOSS_HP_EVERY: Duration = Duration::from_secs(1);
/// A map read longer ago than this isn't passed on (waiting for the server).
const MAP_MAX_AGE_MS: i64 = 10 * 60_000;
/// The same map, unchanged, is passed on again after this long at the earliest.
const MAP_RESEND_SAME_MS: i64 = 5 * 60_000;
/// A pet list is passed on this long after it was read (see the top of this file).
const PET_LIST_SETTLE_MS: i64 = 3_000;
/// Most pet records kept while the server isn't known yet.
const MAX_PENDING_PETS: usize = 256;
/// Most loot records kept until the next SELF_RATE tick passes them on.
const MAX_PENDING_LOOT: usize = 256;
/// The pet records, as `plugins::decoder` specs; each name's event in `pet_event`.
const PET_DECODERS: &str = r#"[
    { "name": "pet-list", "opcode": "00 90", "fields": [
        { "type": "skip:4" },
        { "name": "levels", "type": "list", "count": "varint", "fields": [
            { "name": "pet", "type": "u32" }, { "type": "skip:4" }, { "name": "level", "type": "u32" } ] },
        { "name": "points", "type": "list", "count": "varint", "fields": [
            { "name": "pet", "type": "u32" }, { "name": "points", "type": "u32" } ] } ] },
    { "name": "pet-souls", "opcode": "0D 90", "fields": [
        { "name": "souls", "type": "list", "count": "u8", "fields": [
            { "name": "pet", "type": "u32" }, { "name": "amount", "type": "u32" } ] } ] },
    { "name": "pet-level", "opcode": "14 90", "fields": [
        { "type": "skip:2" }, { "name": "pet", "type": "u32" }, { "name": "level", "type": "u32" } ] }
]"#;
/// The Black Cloud Merchant's shop list (see the top of this file).
const SHOP_OPCODE: [u8; 2] = [0x91, 0x56];
/// One offer of it: listing id u32, item u32, count u64, tier u8, 0x5D, original u64, price u64.
const SHOP_ENTRY_LEN: usize = 34;
const SHOP_ENTRY_MARK: u8 = 0x5D;

/// A shop list's offers (`91 56`, `body` after the opcode: request id u8,
/// count u8, the offers, then the next change as unix ms) — null when none
/// reads. A byte sits between some offers (0x55, 0x05 before the 2nd, 6th,
/// 10th …): skipped, as is anything that doesn't read as one.
fn shop_event(body: &[u8]) -> Option<serde_json::Value> {
    let count = *body.get(1)? as usize;
    let u32_at = |b: &[u8], i: usize| u32::from_le_bytes(b[i..i + 4].try_into().unwrap());
    let u64_at = |b: &[u8], i: usize| u64::from_le_bytes(b[i..i + 8].try_into().unwrap());
    let mut offers = Vec::new();
    let mut tab = None;
    let mut at = 2;
    while offers.len() < count && at + SHOP_ENTRY_LEN <= body.len() {
        let e = &body[at..at + SHOP_ENTRY_LEN];
        let (listing, item, items, tier, original, price) = (u32_at(e, 0), u32_at(e, 4), u64_at(e, 8), e[16], u64_at(e, 18), u64_at(e, 26));
        if e[17] != SHOP_ENTRY_MARK || !(1..=1_000_000).contains(&items) || price == 0 || price > original || original >= 1_000_000_000_000 {
            at += 1;
            continue;
        }
        tab.get_or_insert(listing / 10_000);
        offers.push(serde_json::json!({ "item": item, "count": items, "tier": tier, "price": price, "original": original }));
        at += SHOP_ENTRY_LEN;
    }
    if offers.is_empty() {
        return None;
    }
    let refresh_at = body
        .get(at..at + 8)
        .map(|b| i64::from_le_bytes(b.try_into().unwrap()))
        .filter(|t| (1_600_000_000_000..4_000_000_000_000).contains(t));
    Some(serde_json::json!({ "tab": tab, "refreshAt": refresh_at, "offers": offers }))
}

/// How often the cooldown and cast bars' state is looked at.
const SKILLS_RATE: Duration = Duration::from_millis(100);
/// Where the learned monster cast times are kept between sessions.
const CAST_TIMES_FILE: &str = "cast_times.json";
const CAST_TIMES_SAVE_EVERY: Duration = Duration::from_secs(60);
/// The debuffs a party member can take off, learnt (`learn_cleansable`) and
/// kept between sessions: `[abnormal, …]`.
const CLEANSABLE_FILE: &str = "cleansable.json";
/// Your party as it last was (`save_party`), for the next run: an engine
/// restarted mid-party (a Companion update) hears no roster until someone
/// joins or leaves, and takes it back once a member's `1c 92` shows they are
/// still with you (`DataStorage::note_party_member_seen`).
const PARTY_FILE: &str = "party.json";
/// A saved party older than this is not taken back.
const PARTY_KEEP_MS: i64 = 12 * 3_600_000;

/// The party file's text for this roster — None for no party (fewer than two).
fn party_file_text(roster: &std::collections::HashMap<String, a2tools_dps_meter_lib::combat::data_storage::PartyMember>, now_ms: i64) -> Option<String> {
    if roster.len() < 2 {
        return None;
    }
    let mut members: Vec<_> = roster.iter().collect();
    members.sort_by_key(|(name, m)| (m.slot, (*name).clone()));
    let members: Vec<_> = members
        .into_iter()
        .map(|(name, m)| {
            serde_json::json!({
                "name": name, "slot": m.slot, "level": m.level, "gearScore": m.gear_score, "combatPower": m.combat_power,
                "dbid": m.dbid.to_string(), "class": m.job.map(class_key),
            })
        })
        .collect();
    Some(serde_json::json!({ "savedAt": now_ms, "members": members }).to_string())
}

/// The saved party (see PARTY_FILE), if it is recent enough.
fn read_party_file(text: &str, now_ms: i64) -> Vec<(String, a2tools_dps_meter_lib::combat::data_storage::PartyMember)> {
    use a2tools_dps_meter_lib::combat::data_storage::PartyMember;
    let Ok(saved) = serde_json::from_str::<serde_json::Value>(text) else { return Vec::new() };
    if saved["savedAt"].as_i64().is_none_or(|at| now_ms - at > PARTY_KEEP_MS) {
        return Vec::new();
    }
    saved["members"]
        .as_array()
        .map(|members| {
            members
                .iter()
                .filter_map(|m| {
                    let name = m["name"].as_str()?.to_string();
                    let dbid: u64 = m["dbid"].as_str()?.parse().ok()?;
                    let job = m["class"].as_str().and_then(|key| ALL_CLASSES.into_iter().find(|&c| class_key(c) == key));
                    Some((name, PartyMember {
                        slot: m["slot"].as_u64().unwrap_or(0) as u8,
                        level: m["level"].as_i64().unwrap_or(0) as i32,
                        gear_score: m["gearScore"].as_i64().unwrap_or(0) as i32,
                        combat_power: m["combatPower"].as_i64().unwrap_or(0),
                        server_id: (dbid >> 48) as u16,
                        dbid,
                        job,
                    }))
                })
                .collect()
        })
        .unwrap_or_default()
}
/// Known before anything is learnt: Burn (two kinds) and Stun, which a
/// Cleric's cleanse took off other players on 2026-10-31.
const CLEANSABLE_SEEN: [u32; 3] = [12_000_001, 12_000_121, 12_000_161];
const HEARTBEAT_EVERY: Duration = Duration::from_secs(15);
/// The one-off events (kills, fights, loot, pets …) a reader may fall behind by before they're dropped.
const EVENTS_BUFFER: usize = 1024;
const HEAD_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_HEAD: usize = 8 * 1024;

struct Args {
    data_dir: PathBuf,
    resources: PathBuf,
    language: String,
}

fn parse_args() -> Result<Args, String> {
    let mut data_dir = None;
    let mut resources = None;
    let mut language = "en".to_string();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--data-dir" => data_dir = Some(PathBuf::from(value()?)),
            "--resources" => resources = Some(PathBuf::from(value()?)),
            "--language" => language = value()?,
            other => return Err(format!("unknown argument {other}")),
        }
    }
    Ok(Args {
        data_dir: data_dir.ok_or("--data-dir is required")?,
        resources: resources.ok_or("--resources is required")?,
        language,
    })
}

struct Engine {
    token: String,
    storage: Arc<DataStorage>,
    calculator: Mutex<DpsCalculator>,
    history: FightHistoryManager,
    port_detector: Arc<CombatPortDetector>,
    npcap: AtomicBool,
    capturer: PcapCapturer,
    /// Orders fight snapshots and their writes, as upstream's `fight_save`.
    saving: Mutex<()>,
    /// The latest `DpsData` as JSON.
    dps: watch::Sender<Arc<str>>,
    /// Your character as JSON (`self_json`).
    me: watch::Sender<Arc<str>>,
    /// The players of the other faction in sight as JSON (`pvp_json`).
    pvp: watch::Sender<Arc<str>>,
    /// The game connection's readings as JSON (`net_json`).
    net: watch::Sender<Arc<str>>,
    /// The mobs in sight as JSON (`npcs_json`).
    npcs: watch::Sender<Arc<str>>,
    /// The Hidden Cubes in range as JSON (`cubes_json`).
    cubes: watch::Sender<Arc<str>>,
    /// The cooldown and cast bars' state as JSON (`skills_json`).
    skill_bars: watch::Sender<Arc<str>>,
    /// Your HP, MP, stamina and flight time as JSON (`resources_json`).
    resource_bars: watch::Sender<Arc<str>>,
    /// The party frames' state as JSON (`party_json`).
    party: watch::Sender<Arc<str>>,
    /// The debuffs seen taken off by another player (`learn_cleansable`),
    /// kept for the next session in CLEANSABLE_FILE.
    cleansable: Mutex<HashSet<u32>>,
    /// The entity the meter's view follows (`DpsData::target_id`), 0: none.
    target: AtomicI32,
    /// Icons and names for the bars (`load_bar_data`).
    bar_data: BarData,
    /// One-off events for the stream: (name, JSON).
    events: broadcast::Sender<(&'static str, String)>,
    /// The character name the parent read off the game's screen — put on your
    /// entity once the core has found it, until the game's own self record
    /// names you (`apply_screen_name`).
    screen_name: Mutex<Option<String>>,
    /// The game's round trip, for `self`'s `ping`.
    ping: Arc<PingTracker>,
    /// Skill names, for the death recap.
    skills: Arc<SkillLookup>,
    /// Each character's faction as last known (`<data-dir>/factions.json`):
    /// your self record, which says it, comes only with a zone load — a meter
    /// started in the Abyss would know no enemies until you left
    /// (`remember_faction`).
    factions: Mutex<HashMap<String, u8>>,
    factions_path: PathBuf,
    /// Pet records read (`PET_DECODERS`), waiting to be passed on: when read
    /// (unix ms), the decoder's name, what it read.
    pets: Arc<Mutex<Vec<(i64, String, serde_json::Value)>>>,
    /// Loot read (`capture::loot`), waiting to be passed on: when read (unix ms), what.
    loot: Arc<Mutex<Vec<(i64, Loot)>>>,
    /// Gathering rounds read (`capture::gathering`), waiting to be passed on: when read (unix ms), what.
    gathered: Arc<Mutex<Vec<(i64, Gathered)>>>,
}

/// What the `skills` event names skills and abnormals with: skill icons by
/// a skill's first four digits (`skill_icons.json`), abnormal icons and
/// debuff flags (`abnormals.json`) and abnormal names (`i18n/abnormals`).
#[derive(Default)]
struct BarData {
    skill_icons: HashMap<String, String>,
    abnormal_icons: HashMap<u32, String>,
    abnormal_names: HashMap<u32, String>,
    /// Every abnormal of the game's in `abnormals.json`: whether it files it as a debuff.
    debuffs: HashMap<u32, bool>,
}

impl BarData {
    /// The icon an abnormal shows: `ABNORMAL_ICON_FIXES`' for its skill, else its own.
    fn abnormal_icon(&self, abnormal: u32) -> Option<&str> {
        let digits = abnormal.to_string();
        if digits.len() == 9 {
            if let Some((_, icon)) = ABNORMAL_ICON_FIXES.iter().find(|(skill, _)| digits[..4].parse() == Ok(*skill)) {
                return Some(icon);
            }
        }
        self.abnormal_icons.get(&abnormal).map(String::as_str)
    }

    /// An abnormal with the game's stand-in icon (`PLACEHOLDER_ICONS`).
    fn inner_state(&self, abnormal: u32) -> bool {
        self.abnormal_icons.get(&abnormal).is_some_and(|icon| PLACEHOLDER_ICONS.contains(&icon.as_str()))
    }

    /// An abnormal whose icon says it helps (`BOON_ICON_ENDS`) — never a
    /// debuff, whoever cast it (a monster's own shield on you is none of these).
    fn boon(&self, abnormal: u32) -> bool {
        self.abnormal_icons.get(&abnormal).is_some_and(|icon| BOON_ICON_ENDS.iter().any(|end| icon.ends_with(end)))
    }

    /// The icon a skill shows: `ABNORMAL_ICON_FIXES`' for it, else the data's
    /// (the cooldown bar showed Lunge Stance with the crown, 2026-11-05).
    fn skill_icon(&self, skill: u32) -> Option<&str> {
        let digits = skill.to_string();
        if digits.len() != 8 {
            return None;
        }
        if let Some((_, icon)) = ABNORMAL_ICON_FIXES.iter().find(|(fixed, _)| digits[..4].parse() == Ok(*fixed)) {
            return Some(icon);
        }
        self.skill_icons.get(&digits[..4]).map(String::as_str)
    }
}

/// Skills (first four digits) whose abnormals the game data gives the other
/// one's icon — and since upstream 2.0.87 the skills too (`skill_icons.json`
/// from the game's Skill table), so the two agree and no longer tell. In the
/// game Lunge Stance's buff (114000x01) has the fire, Zikel's Blessing's
/// (112500x01) the crown (checked 2026-10-31 and 2026-11-03).
const ABNORMAL_ICON_FIXES: [(u32, &str); 2] = [(1125, "ICON_GL_SKILL_040"), (1140, "ICON_GL_SKILL_024")];

/// The game's stand-in icons of abnormals that have none of their own — its
/// inner states (Dodge 200 on every dodge, stuns, per-tick helpers): a red
/// and a blue potion that are no potion, kept off the buff bar.
const PLACEHOLDER_ICONS: [&str; 2] = ["ICON_Abnormal_DotHp", "ICON_Abnormal_DotMp"];

/// Icon name endings of abnormals that help (AttackUp, Hot, HpBarrier, …).
const BOON_ICON_ENDS: [&str; 7] = ["Up", "Statup", "Hot", "HpBarrier", "Invincible", "Uplift", "Rebirth_Myself"];

fn load_bar_data(dir: &std::path::Path, language: &str, abnormals: Option<&str>) -> BarData {
    let read = |path: PathBuf| std::fs::read_to_string(path).ok().and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok());
    let ids = |value: Option<serde_json::Value>| -> HashMap<u32, String> {
        value
            .as_ref()
            .and_then(|v| v.as_object())
            .map(|o| o.iter().filter_map(|(k, v)| Some((k.parse().ok()?, v.as_str()?.to_string()))).collect())
            .unwrap_or_default()
    };
    let names = dir.join("i18n").join("abnormals");
    let abnormal_names = match ids(read(names.join(format!("{language}.json")))) {
        found if !found.is_empty() => found,
        _ => ids(read(names.join("en.json"))),
    };
    let table = abnormals
        .and_then(|text| serde_json::from_str::<serde_json::Value>(text).ok())
        .and_then(|v| v.get("abnormals").and_then(|a| a.as_object()).cloned())
        .unwrap_or_default();
    let abnormal_icons: HashMap<u32, String> =
        table.iter().filter_map(|(k, v)| Some((k.parse().ok()?, v.get("icon")?.as_str()?.to_string()))).collect();
    // Only the game's own entries say; the Companion adds icons for some it lacks.
    let debuffs = table.iter().filter_map(|(k, v)| Some((k.parse().ok()?, v.get("debuff")?.as_bool()?))).collect();
    let skill_icons: HashMap<String, String> = read(dir.join("skill_icons.json")).and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
    BarData { skill_icons, abnormal_icons, abnormal_names, debuffs }
}

impl Engine {
    /// The cooldown and cast bars' state (see the top of this file).
    fn skills_json(&self) -> String {
        let name = |skill: u32| Some(self.skills.lookup_skill_name(skill as i32)).filter(|n| !n.is_empty());
        let cooldowns: Vec<_> = self
            .storage
            .skill_cooldowns()
            .into_iter()
            .map(|c| serde_json::json!({ "skill": c.skill, "name": name(c.skill), "icon": self.bar_data.skill_icon(c.skill), "endsAt": c.ends_ms, "totalMs": c.total_ms }))
            .collect();
        let now = a2tools_dps_meter_lib::clock::now_ms();
        let me = self.storage.local_player_id().and_then(|id| i32::try_from(id).ok());
        // One row per abnormal: (abnormal, skill, end, start, stacks, mine,
        // hostile) — its stacks counted, the latest one's timer.
        let mut rows: Vec<(u32, Option<u32>, Option<i64>, i64, u32, bool, bool)> = Vec::new();
        for (instance, end) in self.storage.self_abnormals() {
            if end.is_some_and(|e| e <= now) || self.bar_data.inner_state(instance.abnormal) {
                continue;
            }
            // Ending in weeks: as good as no timer (an account buff).
            let end = end.filter(|&e| e - now <= 86_400_000);
            let mine = Some(instance.caster) == me;
            let hostile = match self.bar_data.debuffs.get(&instance.abnormal) {
                Some(&debuff) => debuff,
                None => !mine && self.storage.is_hostile_caster(instance.caster) && !self.bar_data.boon(instance.abnormal),
            };
            match rows.iter_mut().find(|r| r.0 == instance.abnormal) {
                Some(row) => {
                    row.4 += 1;
                    row.5 |= mine;
                    row.6 |= hostile;
                    if end > row.2 {
                        (row.2, row.3) = (end, instance.timer_start_ms);
                    }
                }
                None => rows.push((instance.abnormal, instance.skill, end, instance.timer_start_ms, 1, mine, hostile)),
            }
        }
        rows.sort_by_key(|r| (r.2.is_none(), r.2, r.0));
        let buffs: Vec<_> = rows
            .into_iter()
            .map(|(abnormal, skill, end, start, stacks, mine, hostile)| {
                let total = end.map(|e| (e - start).max(0));
                serde_json::json!({
                    "abnormal": abnormal, "skill": skill,
                    "name": self.bar_data.abnormal_names.get(&abnormal).cloned().or_else(|| skill.and_then(name)),
                    "icon": self.bar_data.abnormal_icon(abnormal).or_else(|| skill.and_then(|s| self.bar_data.skill_icon(s))),
                    "endsAt": end, "totalMs": total, "stacks": stacks, "mine": mine, "hostile": hostile,
                })
            })
            .collect();
        let entity = self.target.load(Ordering::Relaxed);
        let target = (entity > 0).then(|| {
            let cast = self
                .storage
                .npc_casting(entity)
                .map(|c| serde_json::json!({ "skill": c.skill, "name": name(c.skill), "startedAt": c.started_ms, "castMs": c.cast_ms }));
            serde_json::json!({ "entity": entity, "cast": cast, "deadline": self.storage.boss_deadline(entity) })
        });
        serde_json::json!({ "cooldowns": cooldowns, "buffs": buffs, "target": target }).to_string()
    }

    /// The resource bars' state (see the top of this file).
    fn resources_json(&self) -> String {
        use a2tools_dps_meter_lib::capture::resources::{Gauge, FLIGHT, HP, MP, STAMINA};
        let r = self.storage.self_resources();
        let gauge = |g: Option<Gauge>| g.map(|g| serde_json::json!({ "current": g.current, "max": g.max }));
        serde_json::json!({
            "hp": gauge(r.wide(HP)), "mp": gauge(r.small(MP)),
            "stamina": gauge(r.small(STAMINA)), "flight": gauge(r.small(FLIGHT)),
        })
        .to_string()
    }

    /// Learns which debuffs a party member can take off: one a player's skill
    /// took off someone else (a Cleric's cleanse), not their own escape.
    /// True when it learned a new one.
    fn learn_cleansable(&self) -> bool {
        let mut learned = false;
        for (abnormal, entity, by) in self.storage.take_abnormals_taken_off() {
            // On a player only: an Assassin's marks on a monster are taken off by their own finisher.
            if by == entity || !self.storage.is_player(by) || !self.storage.is_player(entity) || self.bar_data.debuffs.get(&abnormal) != Some(&true) {
                continue;
            }
            learned |= self.cleansable.lock().insert(abnormal);
        }
        learned
    }

    /// The party frames' state (see the top of this file).
    fn party_json(&self) -> String {
        use a2tools_dps_meter_lib::capture::resources::{Gauge, HP, MP};
        let roster = self.storage.get_party_members();
        if roster.len() < 2 {
            return serde_json::json!({ "members": [] }).to_string();
        }
        let me = self.storage.local_player_id().and_then(|id| i32::try_from(id).ok());
        let now = a2tools_dps_meter_lib::clock::now_ms();
        let cleansable = self.cleansable.lock().clone();
        let gauge = |g: Option<Gauge>| g.map(|g| serde_json::json!({ "current": g.current, "max": g.max }));
        let mut members: Vec<_> = roster.into_iter().collect();
        members.sort_by_key(|(name, m)| (m.slot, name.clone()));
        let members: Vec<_> = members
            .into_iter()
            .map(|(name, member)| {
                let id = self.storage.party_member_id(&name);
                let resources = match id {
                    Some(id) if Some(id) == me => self.storage.self_resources(),
                    Some(id) => self.storage.entity_resources(id),
                    None => Default::default(),
                };
                let mut debuffs: Vec<(u32, Option<u32>, Option<i64>, i64, u32)> = Vec::new();
                for (instance, end) in id.map(|id| self.storage.abnormals_on(id)).unwrap_or_default() {
                    if end.is_some_and(|e| e <= now) || self.bar_data.inner_state(instance.abnormal) {
                        continue;
                    }
                    let hostile = match self.bar_data.debuffs.get(&instance.abnormal) {
                        Some(&debuff) => debuff,
                        None => self.storage.is_hostile_caster(instance.caster) && !self.bar_data.boon(instance.abnormal),
                    };
                    if !hostile {
                        continue;
                    }
                    let end = end.filter(|&e| e - now <= 86_400_000);
                    match debuffs.iter_mut().find(|d| d.0 == instance.abnormal) {
                        Some(row) => {
                            row.4 += 1;
                            if end > row.2 {
                                (row.2, row.3) = (end, instance.timer_start_ms);
                            }
                        }
                        None => debuffs.push((instance.abnormal, instance.skill, end, instance.timer_start_ms, 1)),
                    }
                }
                debuffs.sort_by_key(|d| (!cleansable.contains(&d.0), d.2.is_none(), d.2, d.0));
                let debuffs: Vec<_> = debuffs
                    .into_iter()
                    .map(|(abnormal, skill, end, start, stacks)| {
                        serde_json::json!({
                            "abnormal": abnormal,
                            "name": self.bar_data.abnormal_names.get(&abnormal).cloned().or_else(|| skill.map(|s| self.skills.lookup_skill_name(s as i32)).filter(|n| !n.is_empty())),
                            "icon": self.bar_data.abnormal_icon(abnormal).or_else(|| skill.and_then(|s| self.bar_data.skill_icon(s))),
                            "endsAt": end, "totalMs": end.map(|e| (e - start).max(0)), "stacks": stacks,
                            "cleansable": cleansable.contains(&abnormal),
                        })
                    })
                    .collect();
                let hp = gauge(resources.wide(HP));
                serde_json::json!({
                    "id": id, "name": name, "class": member.job.map(class_key),
                    "server": self.storage.stated_server(&name).filter(|&s| s != 0).or((member.server_id != 0).then_some(member.server_id)),
                    "level": (member.level > 0).then_some(member.level), "slot": member.slot,
                    "combatPower": (member.combat_power > 0).then_some(member.combat_power),
                    "itemLevel": (member.gear_score > 0).then_some(member.gear_score),
                    "me": id.is_some() && id == me,
                    "near": id.is_some_and(|id| Some(id) == me || self.storage.player_in_sight(id)),
                    "hp": hp, "mp": gauge(resources.small(MP)),
                    "dead": resources.wide(HP).is_some_and(|g| g.current == 0),
                    "debuffs": debuffs,
                })
            })
            .collect();
        // `listed`: whether the meter lists the party before a fight (see
        // `party_placeholders_wanted`) — the members stay here either way.
        serde_json::json!({ "members": members, "listed": self.storage.party_placeholders_wanted() }).to_string()
    }

    fn save(&self, records: Vec<FightRecord>) {
        for record in records {
            match self.history.save_fight(&record) {
                Ok(()) => {
                    // Whether its target has died yet: the same fight is saved
                    // again every 30 s while it lasts.
                    let killed = self.storage.is_entity_dead(record.target_id);
                    let _ = self.events.send(("fight", serde_json::json!({ "id": record.id, "killed": killed }).to_string()));
                }
                Err(e) => tracing::warn!("Could not save fight {}: {e}", record.id),
            }
        }
    }

    /// The living mobs in sight, by NPC code (see the top of this file).
    fn npcs_json(&self) -> String {
        // Spots in metres, to the nearest MOB_SPOT_STEP_M — a mob shuffling on its spot isn't a new event.
        let metres = |v: f32| (v / 100.0 / MOB_SPOT_STEP_M).round() as i64 * MOB_SPOT_STEP_M as i64;
        let mut spots = self.storage.mob_spots();
        let npcs: Vec<_> = self
            .storage
            .mobs_in_sight(SELF_POS_MAX_AGE_MS)
            .into_iter()
            .map(|(npc, (count, near))| {
                let at: Vec<_> = spots.remove(&npc).unwrap_or_default().iter().map(|p| [metres(p.x), metres(p.y)]).collect();
                serde_json::json!({ "npc": npc, "count": count, "metres": near.map(|m| m.round() as u32), "spots": at })
            })
            .collect();
        let since = a2tools_dps_meter_lib::clock::now_ms() - MOB_KILLS_SHOWN_MS;
        let kills: Vec<_> = self
            .storage
            .mob_kills_since(since)
            .into_iter()
            .map(|(npc, p, at)| serde_json::json!({ "npc": npc, "x": metres(p.x), "y": metres(p.y), "at": at }))
            .collect();
        let target = self.storage.my_target(TARGET_HIT_MS);
        let me = self.storage.self_fix(SELF_POS_MAX_AGE_MS);
        // In whole seconds, so the event isn't new every second for nothing.
        let pos_age = me.and_then(|f| f.age_ms).map(|age| age / 1000 * 1000);
        // Your spot on the ground in metres, to the nearest 2 (else the event is new at every step).
        let pos = me.map(|f| serde_json::json!({ "x": (f.pos.x / 200.0).round() as i64 * 2, "y": (f.pos.y / 200.0).round() as i64 * 2 }));
        let guessed = me.is_some_and(|f| f.guessed);
        // How far off it may be, to 5 m — the spot told's without its age (the app grows that itself).
        let accuracy = me.map(|f| {
            let m = if f.guessed { f.accuracy_m } else { TOLD_ACCURACY_M };
            ((m / 5.0).ceil() as i64 * 5).max(5)
        });
        serde_json::json!({ "npcs": npcs, "kills": kills, "target": target, "posAge": pos_age, "pos": pos, "posGuessed": guessed, "posAccuracy": accuracy }).to_string()
    }

    /// The Hidden Cubes in range (see the top of this file).
    fn cubes_json(&self) -> String {
        let cubes: Vec<_> = self
            .storage
            .hidden_cubes(CUBE_POS_MAX_AGE_MS)
            .into_iter()
            .map(|cube| {
                let (metres, above) = cube.spot.map_or((None, None), |(m, a)| (Some(m.round() as i32), Some(a.round() as i32)));
                // Whole minutes past the first: the JSON changes only that often.
                let age = cube.age_ms.map(|ms| if ms < 60_000 { 0 } else { ms / 60_000 * 60 });
                let opening = cube.opening_by_you.map(|you| if you { "you" } else { "other" });
                serde_json::json!({ "metres": metres, "above": above, "age": age, "opening": opening })
            })
            .collect();
        serde_json::json!({ "cubes": cubes, "opened": self.storage.cubes_opened() }).to_string()
    }

    /// The shop lists read since the last call, as `petShop` JSON — at once,
    /// `server` null while it isn't known (the meter started mid-game).
    fn take_shop_events(&self, server: u16) -> Vec<String> {
        let mut pets = self.pets.lock();
        let (shops, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut *pets).into_iter().partition(|(_, name, _)| name == "pet-shop");
        *pets = rest;
        shops
            .into_iter()
            .filter_map(|(at, _, data)| {
                let mut body = data.as_object()?.clone();
                body.insert("server".into(), if server == 0 { serde_json::Value::Null } else { server.into() });
                body.insert("at".into(), at.into());
                Some(serde_json::Value::Object(body).to_string())
            })
            .collect()
    }

    /// The pet records ready to be passed on, oldest first, as (event, JSON):
    /// everything up to a pet list younger than PET_LIST_SETTLE_MS, which
    /// waits with what came after it.
    fn take_pet_events(&self, server: u16, now: i64) -> Vec<(&'static str, String)> {
        let mut pets = self.pets.lock();
        let ready = pets
            .iter()
            .position(|(at, name, _)| name == "pet-list" && now - at < PET_LIST_SETTLE_MS)
            .unwrap_or(pets.len());
        pets.drain(..ready)
            .filter_map(|(at, name, data)| {
                let mut body = data.as_object()?.clone();
                body.insert("server".into(), server.into());
                body.insert("at".into(), at.into());
                let event = match name.as_str() {
                    "pet-list" => "petList",
                    "pet-souls" => "petSouls",
                    "pet-level" => "petLevel",
                    _ => return None,
                };
                Some((event, serde_json::Value::Object(body).to_string()))
            })
            .collect()
    }

    /// The loot read since the last call, as `loot` events (see the top of this file).
    fn take_loot_events(&self) -> Vec<String> {
        let found = std::mem::take(&mut *self.loot.lock());
        let dungeon = self.storage.current_dungeon_id();
        found
            .into_iter()
            .map(|(at, loot)| {
                let items: Vec<_> = loot.items.iter().map(|&(item, count)| serde_json::json!({ "item": item, "count": count })).collect();
                let run_at = if dungeon > 0 { self.storage.run_started_at(at) } else { 0 };
                serde_json::json!({ "at": at, "source": loot.source.as_str(), "dungeon": dungeon, "runAt": run_at, "items": items }).to_string()
            })
            .collect()
    }

    fn take_gather_events(&self) -> Vec<String> {
        let found = std::mem::take(&mut *self.gathered.lock());
        let map = self.storage.current_map_id();
        found
            .into_iter()
            .map(|(at, g)| {
                let node = self.storage.gathered_node_code(g.node);
                tracing::info!(
                    "Gathered: {} item {} x{} (bonus {}, node {node:?}, map {map:?})",
                    g.outcome.as_str(),
                    g.item,
                    g.count,
                    g.bonus
                );
                serde_json::json!({
                    "at": at, "outcome": g.outcome.as_str(), "item": g.item, "count": g.count,
                    "bonus": g.bonus, "node": node, "map": map,
                })
                .to_string()
            })
            .collect()
    }

    /// Upstream's 30-second save: boss fights that changed since the last one.
    fn save_open_fights(&self) {
        let _saving = self.saving.lock();
        if self.storage.damage_generation() <= 0 {
            return;
        }
        let records = self.calculator.try_lock().map(|mut calc| calc.snapshot_boss_fights());
        if let Some(records) = records {
            self.save(records);
        }
    }

    /// Upstream's `save_fights_before_reset`: everything, before it is cleared.
    fn save_all_fights(&self) {
        let _saving = self.saving.lock();
        if self.storage.damage_generation() <= 0 {
            return;
        }
        let records = self.calculator.lock().snapshot_boss_fights_force();
        self.save(records);
    }

    fn reset(&self) {
        self.save_all_fights();
        self.calculator.lock().restart_target_selection(true);
        self.storage.forget_guessed_nicknames();
        // Upstream's reset does this too: the party who have not fought go
        // until the next roster. Left out, ↻ changed nothing on an empty
        // meter that listed a party (2026-11-07).
        self.storage.hide_party_placeholders();
    }

    /// Who you are playing, as far as the game has said, and your Odyle
    /// energy — with whose it is: the game sends it before naming you.
    fn self_json(&self) -> String {
        let profile = self.storage.local_profile();
        let odyle = self.storage.odyle_energy().map(|(basic, charged)| {
            serde_json::json!({ "basic": basic, "charged": charged, "owner": self.storage.odyle_owner() })
        });
        // Only the name the game's own self record stated: one the parent sent
        // (`screen_name`) or a loot vote picked is no news to the parent.
        let name = profile.name.filter(|_| self.storage.local_identity_from_self_record());
        // Entry counts by the game's id as [first, second, ms] (see
        // `capture::game_lists`), currencies by name (`itemsWhole`: since a
        // login, so one missing is at 0), Duty missions done today, morph
        // counts by recipe (`morphs.whole`: the Morph window's lists, so one
        // missing is at 0) — whose, as the Odyle energy. `dutiesAt` and
        // `morphs.at`: when the game sent them (ms) — older than the reset is stale;
        // `dutiesAtLogin`: with a login, where they may still be yesterday's.
        let lists = self.storage.game_lists();
        let has_morphs = !lists.morphs_server.is_empty() || !lists.morphs_character.is_empty();
        let lists = (!lists.counts.is_empty() || !lists.items.is_empty() || lists.duties.is_some() || has_morphs).then(|| {
            let counts: serde_json::Map<String, serde_json::Value> = lists
                .counts
                .values()
                .map(|e| (e.id.to_string(), serde_json::json!([e.first, e.second, e.ms])))
                .collect();
            let items: serde_json::Map<String, serde_json::Value> = CURRENCIES
                .iter()
                .filter_map(|(id, key)| lists.items.get(id).map(|amount| (key.to_string(), (*amount).into())))
                .collect();
            let by_recipe = |m: &std::collections::BTreeMap<u32, u32>| -> serde_json::Map<String, serde_json::Value> {
                m.iter().map(|(recipe, count)| (recipe.to_string(), (*count).into())).collect()
            };
            let morphs = has_morphs.then(|| {
                serde_json::json!({ "server": by_recipe(&lists.morphs_server), "character": by_recipe(&lists.morphs_character), "whole": lists.morphs_whole, "at": lists.morphs_at })
            });
            serde_json::json!({ "owner": lists.owner, "counts": counts, "items": items, "itemsWhole": lists.items_whole, "duties": lists.duties, "dutiesAt": lists.duties_at, "dutiesAtLogin": lists.duties_at_login, "morphs": morphs })
        });
        serde_json::json!({
            "name": name,
            "serverId": profile.server_id,
            "class": profile.class.map(class_key),
            "level": profile.level,
            "itemLevel": profile.gear_score,
            "combatPower": profile.combat_power,
            "odyle": odyle,
            "lists": lists,
            "ping": self.ping.current_ping_ms(),
            "map": self.storage.current_map_id(),
        })
        .to_string()
    }

    /// A meter started mid-session hears your name only at the next login or
    /// zone load; your entity it finds sooner (your own-stats records), but
    /// leaves nameless — a remembered name may be another character's. The
    /// parent's name was read off the game's menu just now, so it's taken like
    /// one the player typed into the A2Tools meter (its `set_character_name`
    /// with `manual`): on the entity known as you, until the game says itself.
    /// Your faction, once known, is kept for this character; until it is
    /// known, the one it had last time stands in.
    fn remember_faction(&self) {
        let Some(name) = self.storage.local_profile().name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()) else {
            return;
        };
        let mut factions = self.factions.lock();
        match self.storage.self_faction() {
            Some(faction) if factions.get(&name) != Some(&faction) => {
                factions.insert(name, faction);
                if let Ok(text) = serde_json::to_string(&*factions) {
                    let _ = std::fs::write(&self.factions_path, text);
                }
            }
            None => {
                if let Some(&faction) = factions.get(&name) {
                    self.storage.guess_self_faction(faction);
                }
            }
            _ => {}
        }
    }

    fn apply_screen_name(&self) {
        let Some(name) = self.screen_name.lock().clone() else { return };
        if self.storage.local_identity_from_self_record() {
            return;
        }
        if self.storage.local_character_name().as_deref() != Some(name.as_str()) {
            self.storage.set_local_character_name(Some(name.clone()));
        }
        if let Some(id) = self.storage.local_player_id()
            && self.storage.get_nickname(id as i32).as_deref() != Some(name.as_str())
        {
            tracing::info!("screen name '{name}' on entity {id}");
            self.storage.set_permanent_nickname(id as i32, &name);
        }
    }

    /// The PvP view's players (see the top of this file).
    fn net_json(&self) -> String {
        let health = self.capturer.net_health();
        if let Some((at, ping)) = self.ping.latest_ping() {
            health.note_game_ping(at, ping);
        }
        health
            .snapshot(chrono::Utc::now().timestamp_millis())
            .map(|r| {
                let rtt = r.rtt_beside(self.ping.current_ping_ms(), self.ping.held_ping_ms());
                serde_json::json!({ "rtt": rtt, "resentPct": r.resent_pct, "stallMs": r.stall_ms, "serverLagMs": r.server_lag_ms })
            })
            .unwrap_or(serde_json::Value::Null)
            .to_string()
    }

    fn pvp_json(&self) -> String {
        let players: Vec<_> = self
            .storage
            .nearby_enemies()
            .into_iter()
            .map(|p| {
                serde_json::json!({
                    "id": p.id,
                    "name": p.name,
                    "class": p.class.map(class_key),
                    "itemLevel": p.item_level,
                    "rank": p.rank,
                    "serverId": p.server,
                    "paired": p.paired,
                    "seenAt": p.last_seen_ms,
                    "attackedAt": p.attacked_me_ms,
                    "dealt": p.dealt,
                    "taken": p.taken,
                })
            })
            .collect();
        let death = self.storage.last_pvp_death().map(|d| {
            let attackers: Vec<_> = d
                .attackers
                .iter()
                .map(|a| {
                    serde_json::json!({
                        "name": a.name,
                        "class": a.class.map(class_key),
                        "rank": a.rank,
                        "serverId": a.server,
                        "paired": a.paired,
                        "damage": a.damage,
                        "hits": a.hits,
                        "skill": self.skills.lookup_skill_name(a.top_skill),
                    })
                })
                .collect();
            serde_json::json!({
                "at": d.at_ms,
                "killer": d.killer,
                "skill": self.skills.lookup_skill_name(d.killing_skill),
                "abyss": d.abyss,
                "attackers": attackers,
            })
        });
        let kills: Vec<_> = self
            .storage
            .pvp_kills()
            .into_iter()
            .map(|k| {
                serde_json::json!({
                    "at": k.at_ms,
                    "name": k.name,
                    "class": k.class.map(class_key),
                    "itemLevel": k.item_level,
                    "rank": k.rank,
                    "enemyServerId": k.server,
                    "paired": k.paired,
                    "killingBlow": k.killing_blow,
                    "damage": k.damage,
                    "hits": k.hits,
                    "skill": self.skills.lookup_skill_name(k.top_skill),
                    "total": k.total,
                    "helpers": k.helpers,
                    "abyss": k.abyss,
                })
            })
            .collect();
        // Abyss Points gained (`combat::kda`, upstream 2.0.94): the rises of the total.
        let abyss_points: Vec<serde_json::Value> = self
            .storage
            .kda_since(0)
            .map(|(_, events)| events)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|e| (e.kind == KdaKind::Points).then(|| serde_json::json!({ "at": e.at, "amount": e.amount })))
            .collect();
        serde_json::json!({ "players": players, "abyss": self.storage.in_abyss(), "death": death, "kills": kills, "abyssPoints": abyss_points })
            .to_string()
    }

    fn status(&self) -> serde_json::Value {
        serde_json::json!({
            "version": VERSION,
            "npcap": self.npcap.load(Ordering::SeqCst),
            "capturing": self.capturer.is_running(),
            "gamePort": self.port_detector.current_port(),
            "gameWindow": platform::window_detector::find_aion2_window(),
            "gameForeground": platform::window_detector::is_aion2_foreground(),
        })
    }
}

/// Every class, for `class_key` backwards.
const ALL_CLASSES: [JobClass; 9] = [
    JobClass::Gladiator, JobClass::Templar, JobClass::Ranger, JobClass::Assassin, JobClass::Sorcerer,
    JobClass::Cleric, JobClass::Elementalist, JobClass::Chanter, JobClass::Fighter,
];

/// The class as GuildHub names it (its AION2_CLASSES).
fn class_key(class: JobClass) -> &'static str {
    match class {
        JobClass::Gladiator => "gladiator",
        JobClass::Templar => "templar",
        JobClass::Ranger => "ranger",
        JobClass::Assassin => "assassin",
        JobClass::Sorcerer => "sorcerer",
        JobClass::Cleric => "cleric",
        JobClass::Elementalist => "spiritmaster",
        JobClass::Chanter => "chanter",
        JobClass::Fighter => "brawler",
    }
}

fn load_resources(dir: &std::path::Path, language: &str) -> (SkillLookup, NpcLookup, HashSet<i32>, Option<String>) {
    let skills = SkillLookup::new();
    let npcs = NpcLookup::new();
    let mut dot_ids = HashSet::new();
    if !dir.join("i18n").join("skills").exists() {
        tracing::warn!("No game data under {}", dir.display());
        return (skills, npcs, dot_ids, None);
    }
    if let Ok(text) = std::fs::read_to_string(dir.join("dot_skill_ids.json"))
        && let Ok(ids) = serde_json::from_str::<Vec<i32>>(&text)
    {
        dot_ids.extend(ids);
    }
    lookup::load_language(&skills, &npcs, dir, language);
    let abnormals = std::fs::read_to_string(dir.join("abnormals.json")).ok();
    (skills, npcs, dot_ids, abnormals)
}

/// The `bossMap` event of a map: its field bosses named by their NPC code
/// (as `bossKill` names them), the ones the meter doesn't know left out —
/// None when none is left.
fn boss_map_event(server: u16, map: &BossMap) -> Option<serde_json::Value> {
    let bosses: Vec<_> = map
        .bosses
        .iter()
        .filter_map(|b| {
            let npc = npc_of_map_id(b.id)?;
            Some(serde_json::json!({ "npc": npc, "up": b.up, "at": b.time_ms }))
        })
        .collect();
    (!bosses.is_empty()).then(|| serde_json::json!({ "server": server, "map": map.map_id, "seenAt": map.seen_ms, "bosses": bosses }))
}

fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("GH_ENGINE_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = match parse_args() {
        Ok(args) => args,
        Err(e) => {
            eprintln!("guildhub-dps-engine: {e}");
            return std::process::ExitCode::from(2);
        }
    };
    let token = std::env::var("GH_ENGINE_TOKEN").unwrap_or_default();
    if token.len() < 16 {
        eprintln!("guildhub-dps-engine: GH_ENGINE_TOKEN (16+ characters) is required");
        return std::process::ExitCode::from(2);
    }
    let _ = std::fs::create_dir_all(&args.data_dir);
    if std::env::var("GH_ENGINE_SAVE_PACKETS").is_ok_and(|v| v == "1") {
        let dir = args.data_dir.join("packets");
        let _ = std::fs::create_dir_all(&dir);
        a2tools_dps_meter_lib::logging::logger::set_packet_log_enabled(true, &dir);
    }

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("guildhub-dps-engine: no runtime: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    runtime.block_on(run(args, token))
}

async fn run(args: Args, token: String) -> std::process::ExitCode {
    let (skills, npcs, dot_ids, abnormals) = load_resources(&args.resources, &args.language);
    let skills = Arc::new(skills);
    let npcs = Arc::new(npcs);

    let storage = Arc::new(DataStorage::new());
    if let Some(text) = &abnormals {
        storage.set_abnormal_stack_limits(a2tools_dps_meter_lib::capture::abnormal::stack_limits(&text));
    }
    let cast_times_path = args.data_dir.join(CAST_TIMES_FILE);
    if let Some(learned) = std::fs::read_to_string(&cast_times_path).ok().and_then(|text| serde_json::from_str(&text).ok()) {
        storage.set_cast_times(learned);
    }
    let bar_data = load_bar_data(&args.resources, &args.language, abnormals.as_deref());
    let sight_radii_path = args.data_dir.join(SIGHT_RADII_FILE);
    if let Some(radii) = std::fs::read_to_string(&sight_radii_path).ok().and_then(|text| serde_json::from_str(&text).ok()) {
        storage.set_sight_radii(radii);
    }
    let ping = Arc::new(PingTracker::with_perf_clock(platform::clock::perf_clock()));
    let port_detector = Arc::new(CombatPortDetector::new());
    let calculator = DpsCalculator::new(storage.clone(), skills.clone(), npcs.clone(), ping.clone());

    let (tx, rx) = mpsc::channel::<CapturedPayload>(4096);
    let (dps, _) = watch::channel::<Arc<str>>(Arc::from(""));
    let (me, _) = watch::channel::<Arc<str>>(Arc::from(""));
    let (pvp, _) = watch::channel::<Arc<str>>(Arc::from(""));
    let (net, _) = watch::channel::<Arc<str>>(Arc::from(""));
    let (npcs_in_sight, _) = watch::channel::<Arc<str>>(Arc::from(""));
    let (cubes, _) = watch::channel::<Arc<str>>(Arc::from(""));
    let (skill_bars, _) = watch::channel::<Arc<str>>(Arc::from(""));
    let (resource_bars, _) = watch::channel::<Arc<str>>(Arc::from(""));
    let (party, _) = watch::channel::<Arc<str>>(Arc::from(""));
    let party_path = args.data_dir.join(PARTY_FILE);
    if let Ok(text) = std::fs::read_to_string(&party_path) {
        let remembered = read_party_file(&text, a2tools_dps_meter_lib::clock::now_ms());
        if !remembered.is_empty() {
            tracing::info!("Remembered party of {} members, until a member shows up", remembered.len());
            storage.remember_party(remembered);
        }
    }
    let cleansable_path = args.data_dir.join(CLEANSABLE_FILE);
    let mut cleansable: HashSet<u32> = CLEANSABLE_SEEN.into_iter().collect();
    if let Some(learned) = std::fs::read_to_string(&cleansable_path).ok().and_then(|text| serde_json::from_str::<Vec<u32>>(&text).ok()) {
        cleansable.extend(learned);
    }
    // Room for a burst (a cube's loot, a shop's tabs, kills at once) while the app reads slowly — see events().
    let (events, _) = broadcast::channel(EVENTS_BUFFER);
    let engine = Arc::new(Engine {
        token,
        storage: storage.clone(),
        calculator: Mutex::new(calculator),
        history: FightHistoryManager::new(args.data_dir.clone()),
        port_detector: port_detector.clone(),
        npcap: AtomicBool::new(false),
        capturer: PcapCapturer::new(tx, port_detector.clone()),
        saving: Mutex::new(()),
        dps,
        me,
        pvp,
        net,
        npcs: npcs_in_sight,
        cubes,
        skill_bars,
        resource_bars,
        party,
        cleansable: Mutex::new(cleansable),
        target: AtomicI32::new(0),
        bar_data,
        events,
        screen_name: Mutex::new(None),
        ping: ping.clone(),
        skills: skills.clone(),
        factions: Mutex::new(
            std::fs::read_to_string(args.data_dir.join("factions.json"))
                .ok()
                .and_then(|text| serde_json::from_str(&text).ok())
                .unwrap_or_default(),
        ),
        factions_path: args.data_dir.join("factions.json"),
        pets: Arc::new(Mutex::new(Vec::new())),
        loot: Arc::new(Mutex::new(Vec::new())),
        gathered: Arc::new(Mutex::new(Vec::new())),
    });

    // The pet records and your loot, off every frame the parser reads.
    {
        let decoders: Vec<Decoder> = serde_json::from_str::<Vec<DecoderSpec>>(PET_DECODERS)
            .map_err(|e| e.to_string())
            .and_then(|specs| specs.iter().map(Decoder::compile).collect())
            .unwrap_or_else(|e| {
                tracing::warn!("Pet decoders: {e}");
                Vec::new()
            });
        let pets = engine.pets.clone();
        let loot = engine.loot.clone();
        let gathered = engine.gathered.clone();
        let loot_reader = Mutex::new(LootReader::new());
        let events = engine.events.clone();
        frame_tap::set(Some(Arc::new(move |packet: &[u8]| {
            // Not on the SELF_RATE tick: the overlay hides for the window's cursor within a second.
            if let Some(window) = commendation::read(packet) {
                let open = window == Commendation::Opened;
                let _ = events.send(("commendation", serde_json::json!({ "open": open }).to_string()));
            }
            if let Some(round) = gathering::read(packet) {
                let mut gathered = gathered.lock();
                if gathered.len() < MAX_PENDING_LOOT {
                    gathered.push((a2tools_dps_meter_lib::clock::now_ms(), round));
                }
            }
            if let Some(found) = loot_reader.lock().read(packet) {
                let mut loot = loot.lock();
                if loot.len() < MAX_PENDING_LOOT {
                    loot.push((a2tools_dps_meter_lib::clock::now_ms(), found));
                }
            }
            let len = read_varint(packet, 0).length;
            if len <= 0 {
                return;
            }
            let at = len as usize;
            let Some(opcode) = packet.get(at..at + 2) else { return };
            if opcode == SHOP_OPCODE {
                if let Some(data) = shop_event(&packet[at + 2..]) {
                    let mut pets = pets.lock();
                    if pets.len() < MAX_PENDING_PETS {
                        pets.push((a2tools_dps_meter_lib::clock::now_ms(), "pet-shop".to_string(), data));
                    }
                }
                return;
            }
            for d in decoders.iter().filter(|d| d.opcode == opcode) {
                let Some(data) = d.decode(&packet[at + 2..]) else { continue };
                let mut pets = pets.lock();
                if pets.len() < MAX_PENDING_PETS {
                    pets.push((a2tools_dps_meter_lib::clock::now_ms(), d.name.clone(), data));
                }
            }
        })));
    }

    {
        let engine = Arc::downgrade(&engine);
        storage.set_before_reset(move || {
            if let Some(engine) = engine.upgrade() {
                engine.save_all_fights();
            }
        });
    }

    // Capture: now if Npcap is there, else as soon as it is installed.
    {
        let engine = engine.clone();
        tokio::spawn(async move {
            loop {
                if platform::pcap::library_available() {
                    engine.npcap.store(true, Ordering::SeqCst);
                    engine.capturer.start();
                    return;
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });
    }

    let mut dispatcher = CaptureDispatcher::new(storage, skills, npcs, port_detector, ping);
    dispatcher.set_dot_skill_ids(dot_ids);
    dispatcher.use_net_health(engine.capturer.net_health());
    tokio::spawn(async move { dispatcher.run(rx).await });

    // The meter's view, published when it changes.
    {
        let engine = engine.clone();
        // `targetNameEn`: whatever the language, the parent finds a boss's portrait
        // by name, in English or German. Not loaded where the name already is one.
        let npcs_en = (!matches!(args.language.as_str(), "en" | "de"))
            .then(|| std::fs::read_to_string(args.resources.join("i18n").join("npcs").join("en.json")).ok())
            .flatten()
            .map(|text| {
                let npcs = NpcLookup::new();
                npcs.load_from_json(&text);
                Arc::new(npcs)
            });
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(DPS_RATE);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                let calc = engine.clone();
                let npcs_en = npcs_en.clone();
                let json = tokio::task::spawn_blocking(move || {
                    let dps = calc.calculator.lock().get_dps();
                    calc.target.store(dps.target_id, Ordering::Relaxed);
                    // The game sends no HP after the killing blow: the view
                    // would stay on the last reading ("1.7 %"). Whether the
                    // target died is known — said alongside, as targetDead.
                    let dead = dps.target_id > 0 && calc.storage.is_entity_dead(dps.target_id);
                    // The target's stagger gauge now, for the bar under its HP:
                    // {"max","rest","broken"}, null when it has none.
                    let stagger = (dps.target_id > 0)
                        .then(|| calc.storage.stagger_live_gauge(dps.target_id))
                        .flatten()
                        .filter(|&(max, _, _)| max > 0)
                        .map(|(max, rest, broken)| serde_json::json!({ "max": max, "rest": rest, "broken": broken }));
                    let mut view = serde_json::to_value(&dps).unwrap_or_default();
                    // Each player's home server (`server`, null while not
                    // known) — the Companion looks a row's character up by
                    // name and server: a self or loot record's word, else the
                    // party roster's.
                    let roster = calc.storage.get_party_members();
                    if let Some(map) = view.get_mut("map").and_then(|m| m.as_object_mut()) {
                        for actor in map.values_mut() {
                            let Some(fields) = actor.as_object_mut() else { continue };
                            let name = fields.get("nickname").and_then(|n| n.as_str()).unwrap_or("").trim().to_string();
                            let server = (!name.is_empty())
                                .then(|| calc.storage.stated_server(&name).filter(|&s| s != 0).or_else(|| roster.get(&name).map(|m| m.server_id).filter(|&s| s != 0)))
                                .flatten();
                            fields.insert("server".into(), server.into());
                        }
                    }
                    if let Some(fields) = view.as_object_mut() {
                        fields.insert("targetDead".into(), dead.into());
                        fields.insert("targetStagger".into(), stagger.unwrap_or_default());
                        if let Some(npcs) = npcs_en.as_ref().filter(|_| dps.target_mob_code > 0) {
                            fields.insert("targetNameEn".into(), npcs.get_npc_name(dps.target_mob_code).into());
                        }
                    }
                    serde_json::to_string(&view).unwrap_or_default()
                })
                .await
                .unwrap_or_default();
                if !json.is_empty() && **engine.dps.borrow() != *json {
                    engine.dps.send_replace(Arc::from(json));
                }
            }
        });
    }

    // The cooldown and cast bars, published when they change; the learned
    // cast times kept for the next session (CAST_TIMES_FILE).
    {
        let engine = engine.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(SKILLS_RATE);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let (mut seq, mut saved) = (0u64, std::time::Instant::now());
            // Before the first look: the party file stays until a roster says otherwise.
            let mut saved_party = String::from("[]");
            loop {
                interval.tick().await;
                let bars = engine.skills_json();
                if **engine.skill_bars.borrow() != *bars {
                    engine.skill_bars.send_replace(Arc::from(bars));
                }
                let resources = engine.resources_json();
                if **engine.resource_bars.borrow() != *resources {
                    engine.resource_bars.send_replace(Arc::from(resources));
                }
                if engine.learn_cleansable() {
                    let mut learned: Vec<u32> = engine.cleansable.lock().iter().copied().collect();
                    learned.sort_unstable();
                    if let Ok(text) = serde_json::to_string(&learned) {
                        let _ = std::fs::write(&cleansable_path, text);
                    }
                }
                // Your party for the next run, saved as it changes (gone: no party).
                let roster = engine.storage.get_party_members();
                let mut names: Vec<&String> = roster.keys().collect();
                names.sort();
                let key = format!("{names:?}");
                if key != saved_party {
                    saved_party = key;
                    match party_file_text(&roster, a2tools_dps_meter_lib::clock::now_ms()) {
                        Some(text) => {
                            let _ = std::fs::write(&party_path, text);
                        }
                        None => {
                            let _ = std::fs::remove_file(&party_path);
                        }
                    }
                }
                let party = engine.party_json();
                if **engine.party.borrow() != *party {
                    engine.party.send_replace(Arc::from(party));
                }
                if saved.elapsed() >= CAST_TIMES_SAVE_EVERY {
                    saved = std::time::Instant::now();
                    if let Some((next, learned)) = engine.storage.cast_times_since(seq) {
                        seq = next;
                        if let Ok(text) = serde_json::to_string(&learned) {
                            let _ = std::fs::write(&cast_times_path, text);
                        }
                    }
                }
            }
        });
    }

    // Your character and Odyle energy when they change; field boss kills as
    // they happen, and the field bosses of a map the player opened.
    {
        let engine = engine.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(SELF_RATE);
            let mut map_seq = 0u64;
            let mut last_hp = std::time::Instant::now();
            let (mut sight_seq, mut sight_saved) = (0u64, std::time::Instant::now());
            // (server, map) -> when sent, and what it said.
            let mut maps_sent: HashMap<(u16, u32), (i64, String)> = HashMap::new();
            loop {
                interval.tick().await;
                engine.apply_screen_name();
                engine.remember_faction();
                let me = engine.self_json();
                if **engine.me.borrow() != *me {
                    engine.me.send_replace(Arc::from(me));
                }
                let net = engine.net_json();
                if **engine.net.borrow() != *net {
                    engine.net.send_replace(Arc::from(net));
                }
                let pvp = engine.pvp_json();
                if **engine.pvp.borrow() != *pvp {
                    engine.pvp.send_replace(Arc::from(pvp));
                }
                let npcs = engine.npcs_json();
                if **engine.npcs.borrow() != *npcs {
                    engine.npcs.send_replace(Arc::from(npcs));
                }
                let cubes = engine.cubes_json();
                if **engine.cubes.borrow() != *cubes {
                    engine.cubes.send_replace(Arc::from(cubes));
                }
                // How far each mob leaves sight, for the next session (SIGHT_RADII_FILE).
                if sight_saved.elapsed() >= SIGHT_RADII_SAVE_EVERY {
                    if let Some((seq, radii)) = engine.storage.sight_radii_since(sight_seq) {
                        sight_saved = std::time::Instant::now();
                        sight_seq = seq;
                        if let Ok(text) = serde_json::to_string(&radii) {
                            let _ = std::fs::write(&sight_radii_path, text);
                        }
                    }
                }
                let server = engine.storage.local_profile().server_id;
                for event in engine.storage.take_boss_events() {
                    let BossEventKind::Killed { at_ms } = event.kind else { continue };
                    let kill = serde_json::json!({ "npc": event.npc, "at": at_ms, "server": (server != 0).then_some(server) }).to_string();
                    let _ = engine.events.send(("bossKill", kill));
                }
                for body in engine.take_loot_events() {
                    let _ = engine.events.send(("loot", body));
                }
                for body in engine.take_gather_events() {
                    let _ = engine.events.send(("gather", body));
                }
                // The shop lists don't wait for the server: they're about what's on sale now.
                for body in engine.take_shop_events(server) {
                    let _ = engine.events.send(("petShop", body));
                }
                // The server isn't known yet (the meter started mid-game): the maps and pets wait for it.
                if server == 0 {
                    continue;
                }
                for (name, body) in engine.take_pet_events(server, a2tools_dps_meter_lib::clock::now_ms()) {
                    let _ = engine.events.send((name, body));
                }
                // Field boss HP, every BOSS_HP_EVERY: the bosses whose percent
                // changed, and the ones still fought once a minute.
                if last_hp.elapsed() >= BOSS_HP_EVERY {
                    last_hp = std::time::Instant::now();
                    let now = a2tools_dps_meter_lib::clock::now_ms();
                    let bosses: Vec<_> = engine
                        .storage
                        .take_boss_hp(now)
                        .into_iter()
                        .map(|b| serde_json::json!({ "npc": b.npc, "hp": b.percent, "reset": b.reset }))
                        .collect();
                    if !bosses.is_empty() {
                        let body = serde_json::json!({ "server": server, "at": now, "bosses": bosses });
                        let _ = engine.events.send(("bossHp", body.to_string()));
                    }
                }
                let Some((next, maps)) = engine.storage.boss_maps_since(map_seq) else { continue };
                map_seq = next;
                let now = a2tools_dps_meter_lib::clock::now_ms();
                for map in maps.iter().filter(|m| m.header == 0 && now - m.seen_ms <= MAP_MAX_AGE_MS) {
                    let Some(body) = boss_map_event(server, map) else { continue };
                    let said = body["bosses"].to_string();
                    if let Some((at, before)) = maps_sent.get(&(server, map.map_id)) {
                        if *before == said && now - at < MAP_RESEND_SAME_MS {
                            continue;
                        }
                    }
                    maps_sent.insert((server, map.map_id), (now, said));
                    let _ = engine.events.send(("bossMap", body.to_string()));
                }
            }
        });
    }

    // Fights to history every 30 seconds.
    {
        let engine = engine.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(SAVE_EVERY).await;
                let engine = engine.clone();
                let _ = tokio::task::spawn_blocking(move || engine.save_open_fights()).await;
            }
        });
    }

    let listener = match TcpListener::bind(("127.0.0.1", 0)).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("guildhub-dps-engine: cannot listen: {e}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    println!("{}", serde_json::json!({ "event": "ready", "port": port, "version": VERSION }));
    tracing::info!("GuildHub DPS engine {VERSION} listening on 127.0.0.1:{port}");

    // Exit when the parent goes away: its end of our stdin closes.
    let (gone_tx, mut gone_rx) = watch::channel(false);
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            if line.is_err() {
                break;
            }
        }
        let _ = gone_tx.send(true);
    });

    loop {
        tokio::select! {
            _ = gone_rx.changed() => break,
            accepted = listener.accept() => {
                if let Ok((stream, _)) = accepted {
                    let engine = engine.clone();
                    tokio::spawn(async move { handle(stream, engine).await });
                }
            }
        }
    }

    tracing::info!("Parent gone, saving fights and exiting");
    engine.capturer.stop();
    let saving = engine.clone();
    let _ = tokio::task::spawn_blocking(move || saving.save_all_fights()).await;
    std::process::ExitCode::SUCCESS
}

// ── HTTP ────────────────────────────────────────────────────────────────────

struct Request {
    method: String,
    path: String,
    query: String,
    token: String,
}

impl Request {
    fn param(&self, name: &str) -> Option<&str> {
        self.query.split('&').find_map(|pair| pair.strip_prefix(name)?.strip_prefix('='))
    }
}

async fn read_head(stream: &mut TcpStream) -> Option<String> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.windows(4).any(|w| w == b"\r\n\r\n") {
            return String::from_utf8(buf).ok();
        }
        if buf.len() > MAX_HEAD {
            return None;
        }
    }
}

fn parse_request(head: &str) -> Option<Request> {
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_string();
    let target = first.next()?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let mut token = query
        .split('&')
        .find_map(|pair| pair.strip_prefix("token="))
        .unwrap_or("")
        .to_string();
    for line in lines {
        if let Some((name, value)) = line.split_once(':')
            && name.trim().eq_ignore_ascii_case("authorization")
            && let Some(bearer) = value.trim().strip_prefix("Bearer ")
        {
            token = bearer.trim().to_string();
        }
    }
    Some(Request { method, path: path.to_string(), query: query.to_string(), token })
}

/// `%XX` escapes (UTF-8) and `+` for a space — a name in a query.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(b) => {
                        out.push(b);
                        i += 3;
                        continue;
                    }
                    None => out.push(b'%'),
                }
            }
            b'+' => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Constant-time comparison, so the token cannot be guessed byte by byte.
fn token_matches(given: &str, expected: &str) -> bool {
    let (a, b) = (given.as_bytes(), expected.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

async fn respond(stream: &mut TcpStream, status: u16, content_type: &str, body: &[u8]) {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(body).await;
}

async fn json(stream: &mut TcpStream, body: &str) {
    respond(stream, 200, "application/json", body.as_bytes()).await;
}

async fn handle(mut stream: TcpStream, engine: Arc<Engine>) {
    let Ok(Some(head)) = tokio::time::timeout(HEAD_TIMEOUT, read_head(&mut stream)).await else {
        return;
    };
    let Some(req) = parse_request(&head) else {
        respond(&mut stream, 400, "text/plain", b"bad request").await;
        return;
    };
    if !token_matches(&req.token, &engine.token) {
        respond(&mut stream, 403, "text/plain", b"forbidden").await;
        return;
    }

    match (req.method.as_str(), req.path.as_str()) {
        ("GET", "/v1/status") => json(&mut stream, &engine.status().to_string()).await,
        ("GET", "/v1/dps") => {
            let current = engine.dps.borrow().clone();
            json(&mut stream, if current.is_empty() { "null" } else { &current }).await;
        }
        ("GET", "/v1/self") => {
            let current = engine.me.borrow().clone();
            json(&mut stream, if current.is_empty() { "null" } else { &current }).await;
        }
        ("GET", "/v1/events") => events(stream, engine).await,
        ("GET", "/v1/fights") => {
            let e = engine.clone();
            let body = tokio::task::spawn_blocking(move || serde_json::to_string(&e.history.list_fights()))
                .await
                .ok()
                .and_then(Result::ok)
                .unwrap_or_else(|| "[]".into());
            json(&mut stream, &body).await;
        }
        ("GET", path) if path.starts_with("/v1/fights/") => {
            let id = path["/v1/fights/".len()..].to_string();
            if !is_plain_name(&id) {
                respond(&mut stream, 400, "text/plain", b"bad fight id").await;
                return;
            }
            let e = engine.clone();
            let loaded = tokio::task::spawn_blocking(move || e.history.load_fight(&id)).await;
            match loaded {
                Ok(Ok(record)) => json(&mut stream, &serde_json::to_string(&record).unwrap_or_default()).await,
                _ => respond(&mut stream, 404, "text/plain", b"no such fight").await,
            }
        }
        ("POST", "/v1/config") => {
            let mode = req.param("mode").map(str::to_string);
            let window = req.param("window").and_then(|w| w.parse::<i64>().ok());
            {
                let mut calc = engine.calculator.lock();
                if let Some(mode) = &mode {
                    calc.set_target_selection_mode(mode);
                }
                if let Some(ms) = window.filter(|ms| (10_000..=600_000).contains(ms)) {
                    calc.set_all_targets_window_ms(ms);
                }
            }
            json(&mut stream, "{\"ok\":true}").await;
        }
        ("POST", "/v1/character") => {
            let name = req.param("name").map(percent_decode).map(|n| n.trim().to_string()).filter(|n| !n.is_empty() && n.chars().count() <= 32);
            *engine.screen_name.lock() = name;
            engine.apply_screen_name();
            json(&mut stream, "{\"ok\":true}").await;
        }
        ("POST", "/v1/reset") => {
            let e = engine.clone();
            let _ = tokio::task::spawn_blocking(move || e.reset()).await;
            json(&mut stream, "{\"ok\":true}").await;
        }
        (_, "/v1/status" | "/v1/dps" | "/v1/self" | "/v1/events" | "/v1/fights" | "/v1/reset" | "/v1/config" | "/v1/character") => {
            respond(&mut stream, 405, "text/plain", b"method not allowed").await;
        }
        _ => respond(&mut stream, 404, "text/plain", b"not found").await,
    }
}

/// Server-Sent Events (see the top of this file), and a comment now and then
/// to keep the stream open.
async fn events(mut stream: TcpStream, engine: Arc<Engine>) {
    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream; charset=utf-8\r\n\
                Cache-Control: no-store\r\nConnection: close\r\n\r\nretry: 2000\n\n";
    if stream.write_all(head.as_bytes()).await.is_err() {
        return;
    }
    let mut dps = engine.dps.subscribe();
    let mut me = engine.me.subscribe();
    let mut pvp = engine.pvp.subscribe();
    let mut net = engine.net.subscribe();
    let mut npcs = engine.npcs.subscribe();
    let mut cubes = engine.cubes.subscribe();
    let mut skill_bars = engine.skill_bars.subscribe();
    let mut resource_bars = engine.resource_bars.subscribe();
    let mut party = engine.party.subscribe();
    let mut events = engine.events.subscribe();
    let mut beat = tokio::time::interval(HEARTBEAT_EVERY);
    beat.tick().await;
    dps.mark_changed();
    me.mark_changed();
    pvp.mark_changed();
    net.mark_changed();
    npcs.mark_changed();
    cubes.mark_changed();
    skill_bars.mark_changed();
    resource_bars.mark_changed();
    party.mark_changed();
    loop {
        let frame = tokio::select! {
            changed = dps.changed() => {
                if changed.is_err() {
                    return;
                }
                let current = dps.borrow_and_update().clone();
                if current.is_empty() {
                    continue;
                }
                format!("event: dps\ndata: {current}\n\n")
            }
            changed = me.changed() => {
                if changed.is_err() {
                    return;
                }
                let current = me.borrow_and_update().clone();
                if current.is_empty() {
                    continue;
                }
                format!("event: self\ndata: {current}\n\n")
            }
            changed = net.changed() => {
                if changed.is_err() {
                    return;
                }
                let current = net.borrow_and_update().clone();
                if current.is_empty() {
                    continue;
                }
                format!("event: net\ndata: {current}\n\n")
            }
            changed = pvp.changed() => {
                if changed.is_err() {
                    return;
                }
                let current = pvp.borrow_and_update().clone();
                if current.is_empty() {
                    continue;
                }
                format!("event: pvp\ndata: {current}\n\n")
            }
            changed = npcs.changed() => {
                if changed.is_err() {
                    return;
                }
                let current = npcs.borrow_and_update().clone();
                if current.is_empty() {
                    continue;
                }
                format!("event: npcs\ndata: {current}\n\n")
            }
            changed = cubes.changed() => {
                if changed.is_err() {
                    return;
                }
                let current = cubes.borrow_and_update().clone();
                if current.is_empty() {
                    continue;
                }
                format!("event: cubes\ndata: {current}\n\n")
            }
            changed = skill_bars.changed() => {
                if changed.is_err() {
                    return;
                }
                let current = skill_bars.borrow_and_update().clone();
                if current.is_empty() {
                    continue;
                }
                format!("event: skills\ndata: {current}\n\n")
            }
            changed = resource_bars.changed() => {
                if changed.is_err() {
                    return;
                }
                let current = resource_bars.borrow_and_update().clone();
                if current.is_empty() {
                    continue;
                }
                format!("event: resources\ndata: {current}\n\n")
            }
            changed = party.changed() => {
                if changed.is_err() {
                    return;
                }
                let current = party.borrow_and_update().clone();
                if current.is_empty() {
                    continue;
                }
                format!("event: party\ndata: {current}\n\n")
            }
            event = events.recv() => match event {
                Ok((name, data)) => format!("event: {name}\ndata: {data}\n\n"),
                // Fell behind by more than EVENTS_BUFFER: those are gone — at least it's said.
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!("Event stream fell behind: {missed} events dropped");
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => return,
            },
            _ = beat.tick() => ": heartbeat\n\n".to_string(),
        };
        if stream.write_all(frame.as_bytes()).await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use a2tools_dps_meter_lib::capture::boss_map::BossOnMap;

    /// The pet decoders compile, and a soul pickup for two pets (a capture
    /// from a player, 2026-10-19) reads as both.
    #[test]
    fn the_pet_decoders_read_a_soul_pickup() {
        let specs: Vec<DecoderSpec> = serde_json::from_str(PET_DECODERS).unwrap();
        let decoders: Vec<Decoder> = specs.iter().map(|s| Decoder::compile(s).unwrap()).collect();
        assert_eq!(decoders.len(), 3);
        let souls = decoders.iter().find(|d| d.opcode == [0x0D, 0x90]).unwrap();
        let body = [0x02, 0xF7, 0x03, 0, 0, 1, 0, 0, 0, 0x60, 0x04, 0, 0, 1, 0, 0, 0];
        assert_eq!(
            souls.decode(&body).unwrap(),
            serde_json::json!({"souls": [{"pet": 1015, "amount": 1}, {"pet": 1120, "amount": 1}]})
        );
    }

    /// The Pets tab of the Black Cloud Merchant (a capture of ours,
    /// 2026-10-27 15:22): ten pets, the bytes between some offers skipped,
    /// the next change at 16:00.
    #[test]
    fn a_shop_list_reads_its_offers() {
        let hex = "CC0AAB5B030091CADF1F0100000000000000025DA0860100000000007011010000000000558E5B030072CADF1F0100000000000000005DA086010000000000905F010000000000E55B0300EFCADF1F0100000000000000005DA086010000000000905F010000000000CA5B030094CADF1F0100000000000000025DA0860100000000007011010000000000C55B0300ACCADF1F0100000000000000005DA086010000000000905F01000000000055FF5B0300B5CADF1F0100000000000000015DA0860100000000008038010000000000225C030016CBDF1F0100000000000000005DA086010000000000905F010000000000DA5B0300E4CADF1F0100000000000000015DA0860100000000008038010000000000635B030043CADF1F0100000000000000025DA0860100000000007011010000000000058F5B030073CADF1F0100000000000000025DA08601000000000070110100000000008015E07DA101000000000000";
        let body: Vec<u8> = (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect();
        let event = shop_event(&body).unwrap();
        assert_eq!(event["tab"], 22);
        assert_eq!(event["refreshAt"], 1_793_113_200_000_i64);
        let offers = event["offers"].as_array().unwrap();
        assert_eq!(offers.len(), 10);
        assert_eq!(offers[0], serde_json::json!({ "item": 534760081, "count": 1, "tier": 2, "price": 70000, "original": 100000 }));
        assert_eq!(offers[9]["item"], 534760051);
        assert!(shop_event(&body[..20]).is_none());
    }

    /// Lunge Stance's and Zikel's Blessing's abnormals show the icon the
    /// game shows, whatever the data says; the rest show their own.
    #[test]
    fn swapped_stance_abnormals_get_the_games_icon() {
        let abnormal_icons: HashMap<u32, String> = [
            (112500101, "ICON_GL_SKILL_024"), (114000001, "ICON_GL_SKILL_040"),
            (177600001, "ICON_CL_SKILL_Passive_006"), (10012401, "ICON_GL_SKILL_040"),
        ]
        .into_iter()
        .map(|(k, v)| (k, v.to_string()))
        .collect();
        let data = BarData { abnormal_icons, ..Default::default() };
        assert_eq!(data.abnormal_icon(114000001), Some("ICON_GL_SKILL_024"));
        assert_eq!(data.abnormal_icon(112500101), Some("ICON_GL_SKILL_040"));
        assert_eq!(data.abnormal_icon(177600001), Some("ICON_CL_SKILL_Passive_006"));
        assert_eq!(data.abnormal_icon(10012401), Some("ICON_GL_SKILL_040"));
    }

    /// Against the shipped game data: every Lunge Stance abnormal and its
    /// skill show the fire (024), every Zikel's Blessing the crown (040).
    #[test]
    fn the_shipped_data_gives_the_stances_the_games_icons() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("src").join("data");
        let abnormals = std::fs::read_to_string(dir.join("abnormals.json")).unwrap();
        let data = load_bar_data(&dir, "en", Some(&abnormals));
        for level in 0..6 {
            assert_eq!(data.abnormal_icon(114_000_001 + level * 100), Some("ICON_GL_SKILL_024"), "Lunge Stance");
            assert_eq!(data.abnormal_icon(112_500_001 + level * 100), Some("ICON_GL_SKILL_040"), "Zikel's Blessing");
        }
        assert_eq!(data.skill_icon(11_400_000), Some("ICON_GL_SKILL_024"));
        assert_eq!(data.skill_icon(11_250_000), Some("ICON_GL_SKILL_040"));
        assert_eq!(data.abnormal_names.get(&114_000_001).map(String::as_str), Some("Lunge Stance"));
        assert_eq!(data.abnormal_names.get(&112_500_001).map(String::as_str), Some("Zikel's Blessing"));
    }

    /// The same two on the cooldown bar: the skill table swaps them as well.
    #[test]
    fn swapped_stance_skills_get_the_games_icon() {
        let skill_icons: HashMap<String, String> = [("1125", "ICON_GL_SKILL_024"), ("1140", "ICON_GL_SKILL_040"), ("1141", "ICON_GL_SKILL_041")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let data = BarData { skill_icons, ..Default::default() };
        assert_eq!(data.skill_icon(11400000), Some("ICON_GL_SKILL_024"));
        assert_eq!(data.skill_icon(11250000), Some("ICON_GL_SKILL_040"));
        assert_eq!(data.skill_icon(11410000), Some("ICON_GL_SKILL_041"));
        assert_eq!(data.skill_icon(1140), None);
    }

    #[test]
    fn a_map_event_names_the_bosses_by_their_npc_code() {
        let map = BossMap {
            map_id: 1010,
            header: 0,
            seen_ms: 1_791_993_638_495,
            bosses: vec![
                BossOnMap { id: 101_006, up: true, time_ms: 1_791_993_400_000 },
                BossOnMap { id: 111_024, up: false, time_ms: 1_791_994_085_000 },
                BossOnMap { id: 9_999_999, up: false, time_ms: 0 },
            ],
        };
        let event = boss_map_event(2313, &map).unwrap();
        assert_eq!(event["server"], 2313);
        assert_eq!(event["seenAt"], 1_791_993_638_495_i64);
        assert_eq!(event["bosses"], serde_json::json!([
            { "npc": 2100079, "up": true, "at": 1_791_993_400_000_i64 },
            { "npc": 2400855, "up": false, "at": 1_791_994_085_000_i64 },
        ]));
        let unknown = BossMap { bosses: vec![BossOnMap { id: 9_999_999, up: true, time_ms: 0 }], ..map };
        assert!(boss_map_event(2313, &unknown).is_none());
    }
}
