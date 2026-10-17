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
//! | `GET  /v1/dps`         | the meter's current view (`DpsData`, plus `targetDead`) |
//! | `GET  /v1/self`        | your character, its Odyle energy, the ping      |
//! | `GET  /v1/events`      | SSE, see below                                  |
//! | `GET  /v1/fights`      | saved fights (`FightSummary[]`)                 |
//! | `GET  /v1/fights/<id>` | one saved fight (`FightRecord`)                 |
//! | `POST /v1/reset`       | save the open fights, then clear the meter      |
//! | `POST /v1/config?mode=<id>&window=<ms>` | which targets the meter follows (`TargetSelectionMode` ids) and the all-targets window |
//! | `POST /v1/character?name=<name>` | the character played, as the parent read it off the game's screen just now |
//!
//! Events: `dps` (the view) and `self` (as `/v1/self`) on every change, each
//! sent once on connecting (`self` also carries the game's `ping` in ms);
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
//! named it).

use std::collections::{HashMap, HashSet};
use std::io::BufRead;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, mpsc, watch};

use a2tools_dps_meter_lib::capture::boss_map::BossMap;
use a2tools_dps_meter_lib::capture::captured_payload::CapturedPayload;
use a2tools_dps_meter_lib::capture::field_bosses::FIELD_BOSSES;
use a2tools_dps_meter_lib::capture::combat_port_detector::CombatPortDetector;
use a2tools_dps_meter_lib::capture::pcap_capturer::PcapCapturer;
use a2tools_dps_meter_lib::combat::capture_dispatcher::CaptureDispatcher;
use a2tools_dps_meter_lib::combat::data_storage::DataStorage;
use a2tools_dps_meter_lib::combat::dps_calculator::DpsCalculator;
use a2tools_dps_meter_lib::combat::ping_tracker::PingTracker;
use a2tools_dps_meter_lib::combat::data_storage::BossEventKind;
use a2tools_dps_meter_lib::entity::fight_record::FightRecord;
use a2tools_dps_meter_lib::entity::job_class::JobClass;
use a2tools_dps_meter_lib::history::fight_history::{FightHistoryManager, is_plain_name};
use a2tools_dps_meter_lib::i18n::lookup::{self, NpcLookup, SkillLookup};
use a2tools_dps_meter_lib::platform;

const VERSION: &str = env!("CARGO_PKG_VERSION");
/// How often the meter's view is recalculated (upstream's default rate).
const DPS_RATE: Duration = Duration::from_millis(200);
/// How often open boss fights are written to history (as upstream does).
const SAVE_EVERY: Duration = Duration::from_secs(30);
/// How often your character, Odyle energy and field boss kills are looked at.
const SELF_RATE: Duration = Duration::from_secs(1);
/// How often field boss HP is looked at (as upstream's sender does).
const BOSS_HP_EVERY: Duration = Duration::from_secs(3);
/// A map read longer ago than this isn't passed on (waiting for the server).
const MAP_MAX_AGE_MS: i64 = 10 * 60_000;
/// The same map, unchanged, is passed on again after this long at the earliest.
const MAP_RESEND_SAME_MS: i64 = 5 * 60_000;
const HEARTBEAT_EVERY: Duration = Duration::from_secs(15);
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
    /// One-off events for the stream: (name, JSON).
    events: broadcast::Sender<(&'static str, String)>,
    /// The character name the parent read off the game's screen — put on your
    /// entity once the core has found it, until the game's own self record
    /// names you (`apply_screen_name`).
    screen_name: Mutex<Option<String>>,
    /// The game's round trip, for `self`'s `ping`.
    ping: Arc<PingTracker>,
}

impl Engine {
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
        serde_json::json!({
            "name": name,
            "serverId": profile.server_id,
            "class": profile.class.map(class_key),
            "level": profile.level,
            "odyle": odyle,
            "ping": self.ping.current_ping_ms(),
        })
        .to_string()
    }

    /// A meter started mid-session hears your name only at the next login or
    /// zone load; your entity it finds sooner (your own-stats records), but
    /// leaves nameless — a remembered name may be another character's. The
    /// parent's name was read off the game's menu just now, so it's taken like
    /// one the player typed into the A2Tools meter (its `set_character_name`
    /// with `manual`): on the entity known as you, until the game says itself.
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
            let &(npc, _) = FIELD_BOSSES.iter().find(|&&(_, id)| id == b.id)?;
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
    if let Some(text) = abnormals {
        storage.set_abnormal_stack_limits(a2tools_dps_meter_lib::capture::abnormal::stack_limits(&text));
    }
    let ping = Arc::new(PingTracker::with_perf_clock(platform::clock::perf_clock()));
    let port_detector = Arc::new(CombatPortDetector::new());
    let calculator = DpsCalculator::new(storage.clone(), skills.clone(), npcs.clone(), ping.clone());

    let (tx, rx) = mpsc::channel::<CapturedPayload>(4096);
    let (dps, _) = watch::channel::<Arc<str>>(Arc::from(""));
    let (me, _) = watch::channel::<Arc<str>>(Arc::from(""));
    let (events, _) = broadcast::channel(64);
    let engine = Arc::new(Engine {
        token,
        storage: storage.clone(),
        calculator: Mutex::new(calculator),
        history: FightHistoryManager::new(args.data_dir.clone()),
        port_detector: port_detector.clone(),
        npcap: AtomicBool::new(false),
        capturer: PcapCapturer::new(tx),
        saving: Mutex::new(()),
        dps,
        me,
        events,
        screen_name: Mutex::new(None),
        ping: ping.clone(),
    });

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
    tokio::spawn(async move { dispatcher.run(rx).await });

    // The meter's view, published when it changes.
    {
        let engine = engine.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(DPS_RATE);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                let calc = engine.clone();
                let json = tokio::task::spawn_blocking(move || {
                    let dps = calc.calculator.lock().get_dps();
                    // The game sends no HP after the killing blow: the view
                    // would stay on the last reading ("1.7 %"). Whether the
                    // target died is known — said alongside, as targetDead.
                    let dead = dps.target_id > 0 && calc.storage.is_entity_dead(dps.target_id);
                    let mut view = serde_json::to_value(&dps).unwrap_or_default();
                    if let Some(fields) = view.as_object_mut() {
                        fields.insert("targetDead".into(), dead.into());
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

    // Your character and Odyle energy when they change; field boss kills as
    // they happen, and the field bosses of a map the player opened.
    {
        let engine = engine.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(SELF_RATE);
            let mut map_seq = 0u64;
            let mut last_hp = std::time::Instant::now();
            // (server, map) -> when sent, and what it said.
            let mut maps_sent: HashMap<(u16, u32), (i64, String)> = HashMap::new();
            loop {
                interval.tick().await;
                engine.apply_screen_name();
                let me = engine.self_json();
                if **engine.me.borrow() != *me {
                    engine.me.send_replace(Arc::from(me));
                }
                let server = engine.storage.local_profile().server_id;
                for event in engine.storage.take_boss_events() {
                    let BossEventKind::Killed { at_ms } = event.kind;
                    let kill = serde_json::json!({ "npc": event.npc, "at": at_ms, "server": (server != 0).then_some(server) }).to_string();
                    let _ = engine.events.send(("bossKill", kill));
                }
                // The server isn't known yet (the meter started mid-game): the maps wait for it.
                if server == 0 {
                    continue;
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
    let mut events = engine.events.subscribe();
    let mut beat = tokio::time::interval(HEARTBEAT_EVERY);
    beat.tick().await;
    dps.mark_changed();
    me.mark_changed();
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
            event = events.recv() => match event {
                Ok((name, data)) => format!("event: {name}\ndata: {data}\n\n"),
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
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
