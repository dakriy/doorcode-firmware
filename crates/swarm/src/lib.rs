//! `doorcode-swarm`: many desktop doors against one doorcode server. Each
//! door is the real application on its own thread with its own desktop
//! connectors. The swarm enrolls and approves every door through the
//! server's API, sets up one user whose code every typing door allows,
//! types that code at each typing door at a steady rate, opens and closes
//! each traffic door at a steady rate, and reports how the server kept up.
//! See `docs/testing.md`.

pub mod cleanup;
mod door;
mod recorder;
pub mod report;
mod server;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use connectors::Random;
use connectors_host::OsRandom;
use serde_json::{json, Value};
use tempfile::TempDir;

use door::{Door, Running};
use report::{Bytes, DoorEvents, DoorRecord, Measured, Moved, Opened, Summary};
use server::Server;

/// How long every door may take to ask to enroll, and to come online.
const PATIENCE: Duration = Duration::from_secs(60);
/// How long the doors may take to come back after a storm.
const STORM_PATIENCE: Duration = Duration::from_secs(120);
/// How long replies to the last entries may take before the count.
const GRACE: Duration = Duration::from_secs(5);
/// How often a wait looks again.
const POLL: Duration = Duration::from_millis(200);
/// The longest sleep between looks at whether the run was interrupted.
const TICK: Duration = Duration::from_millis(50);
/// The schedule the server has built in that holds at all times.
const ALWAYS_SCHEDULE: u64 = 1;

/// How a load run goes.
#[derive(Debug, Clone)]
pub struct Options {
    pub server: String,
    pub token: String,
    /// The doors that type.
    pub doors: usize,
    /// Entries per typing door per minute.
    pub rate: u32,
    /// The doors that open and close instead of typing.
    pub traffic_doors: usize,
    /// Openings per traffic door per minute.
    pub traffic_rate: u32,
    /// How long each opening stays open.
    pub open_for: Duration,
    /// Leaves the traffic doors locked, so every opening is forced.
    pub forced: bool,
    /// How long the doors are typed at and opened.
    pub duration: Duration,
    /// How long the doors sit idle before typing starts.
    pub idle: Duration,
    /// The provisioning file; `None` downloads it from the server.
    pub provisioning: Option<PathBuf>,
    /// The broker host to connect to instead of the provisioning file's.
    pub broker_host: Option<String>,
    /// Drops every connection at once halfway through the typing.
    pub storm: bool,
    /// Leaves what the run created on the server.
    pub keep: bool,
}

/// What the run created on the server, to delete again.
#[derive(Debug, Default)]
struct Created {
    doors: Vec<u64>,
    zone: Option<u64>,
    /// The zone that puts the traffic doors in open mode.
    open_zone: Option<u64>,
    group: Option<u64>,
    rule: Option<u64>,
    user: Option<u64>,
}

/// Runs the swarm, deletes what it created unless `keep`, also when it
/// fails or is interrupted, and returns what it measured.
pub fn run(options: &Options) -> Result<Summary, String> {
    let interrupted = interrupt_flag()?;
    let server = Server::new(&options.server, &options.token);
    let count = options.doors + options.traffic_doors;
    let swarm = Swarm::start(count, &provisioning(&server, options)?)?;
    let mut created = Created::default();
    let mut typing = swarm.doors();
    let traffic = typing.split_off(options.doors);
    let load = Load {
        server: &server,
        typing,
        traffic,
        options,
        interrupted,
    };
    let result = load.drive(&mut created);
    swarm.stop();
    finish(&server, &created, options.keep);
    result
}

/// The running doors, and the directory that holds their data until the
/// swarm ends.
struct Swarm {
    _work: TempDir,
    doors: Vec<Running>,
}

impl Swarm {
    fn start(count: usize, provisioning: &str) -> Result<Swarm, String> {
        let work = tempfile::tempdir().map_err(|error| error.to_string())?;
        let start =
            |index| Running::start(&work.path().join(format!("door-{index}")), provisioning);
        let doors = (0..count).map(start).collect::<Result<_, _>>()?;
        Ok(Swarm { _work: work, doors })
    }

    fn doors(&self) -> Vec<Arc<Door>> {
        self.doors
            .iter()
            .map(|running| Arc::clone(&running.door))
            .collect()
    }

    fn stop(self) {
        for running in self.doors {
            let name = running.door.hardware_id.clone();
            if let Err(error) = running.stop() {
                eprintln!("door {name} ended: {error}");
            }
        }
    }
}

/// A flag Ctrl-C raises.
fn interrupt_flag() -> Result<Arc<AtomicBool>, String> {
    let flag = Arc::new(AtomicBool::new(false));
    let raised = Arc::clone(&flag);
    ctrlc::set_handler(move || raised.store(true, Ordering::SeqCst))
        .map_err(|error| error.to_string())?;
    Ok(flag)
}

/// The provisioning file the doors start from, with the broker host
/// replaced when asked.
fn provisioning(server: &Server, options: &Options) -> Result<String, String> {
    let mut file = match &options.provisioning {
        Some(path) => read_json(path),
        None => server.get("/firmware/provisioning"),
    }?;
    if let Some(host) = &options.broker_host {
        file["broker"]["host"] = host.as_str().into();
    }
    Ok(file.to_string())
}

fn read_json(path: &Path) -> Result<Value, String> {
    let at = |error: &dyn std::fmt::Display| format!("{}: {error}", path.display());
    let text = fs::read_to_string(path).map_err(|error| at(&error))?;
    serde_json::from_str(&text).map_err(|error| at(&error))
}

/// Deletes what the run created, the doors and the user for good, or says
/// what was kept. The user only types its code, so it made no override and
/// uploaded no firmware that would stop it going for good.
fn finish(server: &Server, created: &Created, keep: bool) {
    if keep {
        eprintln!("kept: {created:?}");
        return;
    }
    remove(server, "/access-rules", created.rule);
    for door in &created.doors {
        remove_for_good(server, "/doors", Some(*door));
    }
    remove(server, "/zones", created.zone);
    remove(server, "/zones", created.open_zone);
    remove(server, "/groups", created.group);
    remove_for_good(server, "/users", created.user);
}

fn remove(server: &Server, kind: &str, id: Option<u64>) {
    let Some(id) = id else { return };
    if let Err(error) = server.delete(&format!("{kind}/{id}")) {
        eprintln!("cleanup: {error}");
    }
}

fn remove_for_good(server: &Server, kind: &str, id: Option<u64>) {
    let Some(id) = id else { return };
    let call = |method: &str, path: &str| server.status(method, path);
    if let Some(kept) = cleanup::delete_for_good(&format!("{kind}/{id}"), call) {
        eprintln!("cleanup: {kept}");
    }
}

/// The id in an answer that created something.
fn id(answer: &Value) -> Result<u64, String> {
    let id = answer["id"].as_u64();
    id.ok_or_else(|| format!("the server answered no id: {answer}"))
}

/// A number of random bytes as lowercase hex.
fn random_hex(bytes: usize) -> String {
    let bytes = OsRandom.bytes(bytes);
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// A code of eight digits.
fn random_code() -> String {
    let digits = OsRandom.bytes(7);
    let digits: String = digits
        .iter()
        .map(|byte| char::from(b'0' + byte % 10))
        .collect();
    format!("7{digits}")
}

/// A random fraction from 0 up to 1.
fn fraction() -> f64 {
    let bytes = OsRandom.bytes(2);
    f64::from(u16::from_le_bytes([bytes[0], bytes[1]])) / 65536.0
}

/// Sleeps until `deadline`; fails when the run is interrupted first.
fn pause_until(deadline: Instant, interrupted: &AtomicBool) -> Result<(), String> {
    loop {
        if interrupted.load(Ordering::SeqCst) {
            return Err("interrupted".to_owned());
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ok(());
        }
        thread::sleep(left.min(TICK));
    }
}

/// Does `act` every `interval` on average until `end`, or until it fails,
/// and returns how many times it began. The first time comes anywhere in
/// the first interval and each gap is half to one and a half intervals, so
/// the doors do not act in lockstep; an act that outlasts its gap is
/// followed at once.
fn repeat(
    interval: Duration,
    end: Instant,
    interrupted: &AtomicBool,
    mut act: impl FnMut() -> Result<(), String>,
) -> u64 {
    let mut times = 0;
    let mut at = Instant::now() + interval.mul_f64(fraction());
    while at < end && pause_until(at, interrupted).is_ok() {
        times += 1;
        if act().is_err() {
            break;
        }
        at = (at + interval.mul_f64(0.5 + fraction())).max(Instant::now());
    }
    times
}

/// Opens `door` for `open_for` and closes it again, at once when the run is
/// interrupted.
fn open_once(door: &Door, open_for: Duration, interrupted: &AtomicBool) -> Result<(), String> {
    door.set_open(true);
    let held = pause_until(Instant::now() + open_for, interrupted);
    door.set_open(false);
    held
}

/// What each kind of door did on its own thread, added up.
fn joined(workers: Vec<thread::JoinHandle<u64>>) -> u64 {
    let counts = workers.into_iter().map(|worker| worker.join().unwrap_or(0));
    counts.sum()
}

/// The MQTT bytes `doors` moved so far.
fn bytes(doors: &[Arc<Door>]) -> Bytes {
    let counts = doors.iter().map(|door| door.bytes());
    counts.fold(Bytes::default(), Bytes::plus)
}

fn records(doors: &[Arc<Door>]) -> Vec<DoorRecord> {
    doors.iter().map(|door| door.record().seen).collect()
}

/// The bytes each kind of door moved over a stretch of the run.
struct Window {
    typing: Moved,
    traffic: Moved,
}

/// What the typing and the opening measured.
struct Typed {
    entries: u64,
    openings: u64,
    moved: Window,
    /// When every connection was dropped, in a storm.
    cut: Option<Instant>,
}

/// A run in progress.
struct Load<'a> {
    server: &'a Server,
    typing: Vec<Arc<Door>>,
    traffic: Vec<Arc<Door>>,
    options: &'a Options,
    interrupted: Arc<AtomicBool>,
}

impl Load<'_> {
    fn drive(&self, created: &mut Created) -> Result<Summary, String> {
        let code = self.prepare(created)?;
        let idle = self.idle()?;
        let typed = self.typed(&code)?;
        let opened = self.opened(idle.traffic, &typed, created)?;
        Ok(self.summary(idle.typing, &typed, opened))
    }

    /// Enrolls the doors and gives them their zones; returns the code the
    /// typing doors allow.
    fn prepare(&self, created: &mut Created) -> Result<String, String> {
        self.enroll(created)?;
        let code = self.access(created)?;
        self.open_mode(created)?;
        Ok(code)
    }

    /// Whether the traffic doors are to be in open mode: there are some,
    /// and they are not to be forced.
    fn opens_in_open_mode(&self) -> bool {
        !self.traffic.is_empty() && !self.options.forced
    }

    /// Every door, typing doors first.
    fn all(&self) -> impl Iterator<Item = &Arc<Door>> {
        self.typing.iter().chain(&self.traffic)
    }

    /// Waits for every door to ask to enroll, approves each, and waits for
    /// every door to come online.
    fn enroll(&self, created: &mut Created) -> Result<(), String> {
        self.wait_for("every door to ask to enroll", PATIENCE, || {
            self.all_enrolling()
        })?;
        self.approve(created)?;
        self.wait_for("every door to come online", PATIENCE, || {
            Ok(self.all_online())
        })
    }

    /// Types the code, waits for the last replies, and after a storm waits
    /// for every door to come back.
    fn typed(&self, code: &str) -> Result<Typed, String> {
        let typed = self.type_code(code)?;
        pause_until(Instant::now() + GRACE, &self.interrupted)?;
        self.recovered(typed.cut)?;
        Ok(typed)
    }

    /// After a storm at `cut`, waits for every door to be online again,
    /// for as long as `STORM_PATIENCE` from the cut.
    fn recovered(&self, cut: Option<Instant>) -> Result<(), String> {
        let Some(cut) = cut else { return Ok(()) };
        self.wait(cut + STORM_PATIENCE, || Ok(self.all_online()))
            .map(drop)
    }

    /// Posts `body` to `path`, and returns the id of what it created.
    fn create(&self, path: &str, body: &Value) -> Result<u64, String> {
        id(&self.server.post(path, body)?)
    }

    /// Waits until `met` holds, looking again every `POLL`, until
    /// `deadline`; returns whether it held.
    fn wait(
        &self,
        deadline: Instant,
        met: impl Fn() -> Result<bool, String>,
    ) -> Result<bool, String> {
        while !met()? {
            if Instant::now() >= deadline {
                return Ok(false);
            }
            pause_until(Instant::now() + POLL, &self.interrupted)?;
        }
        Ok(true)
    }

    /// Waits until `met` holds, and fails when it does not within `patience`.
    fn wait_for(
        &self,
        what: &str,
        patience: Duration,
        met: impl Fn() -> Result<bool, String>,
    ) -> Result<(), String> {
        let held = self.wait(Instant::now() + patience, met)?;
        held.then_some(())
            .ok_or_else(|| format!("timed out waiting for {what}"))
    }

    fn all_enrolling(&self) -> Result<bool, String> {
        let answer = self.server.get("/enrollments")?;
        let listed = answer["enrollments"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let listed: Vec<&str> = listed
            .iter()
            .filter_map(|item| item["hardware_id"].as_str())
            .collect();
        Ok(self
            .all()
            .all(|door| listed.contains(&door.hardware_id.as_str())))
    }

    fn all_online(&self) -> bool {
        self.all().all(|door| door.record().online())
    }

    /// Approves each door as a new door made for it, typing doors first.
    fn approve(&self, created: &mut Created) -> Result<(), String> {
        for door in self.all() {
            let name = format!("swarm-{}", door.hardware_id);
            let made = json!({"name": name, "notes": "Made by doorcode-swarm."});
            let door_id = self.create("/doors", &made)?;
            created.doors.push(door_id);
            let approve = format!("/enrollments/{}/approve", door.hardware_id);
            self.server.post(&approve, &json!({ "door_id": door_id }))?;
        }
        Ok(())
    }

    /// A zone of every typing door, a group allowed into it at all times,
    /// and a user in the group with a new code; returns the code.
    fn access(&self, created: &mut Created) -> Result<String, String> {
        let name = format!("swarm-{}", random_hex(6));
        let doors = &created.doors[..self.typing.len()];
        let zone = self.create("/zones", &json!({"name": name, "door_ids": doors}))?;
        created.zone = Some(zone);
        let group = self.create("/groups", &json!({"slug": name, "name": name}))?;
        created.group = Some(group);
        let rule = json!({"group_id": group, "schedule_id": ALWAYS_SCHEDULE, "zone_ids": [zone]});
        created.rule = Some(self.create("/access-rules", &rule)?);
        let user = json!({"email": format!("{name}@example.com"),
                          "first_name": "Swarm", "last_name": "Load"});
        let user = self.create("/users", &user)?;
        created.user = Some(user);
        self.member(user, group)
    }

    /// Gives `user` a new code and puts it in `group`; returns the code.
    fn member(&self, user: u64, group: u64) -> Result<String, String> {
        let code = random_code();
        self.server.put(
            &format!("/users/{user}/code"),
            Some(&json!({ "code": code })),
        )?;
        self.server
            .put(&format!("/groups/{group}/members/{user}"), None)?;
        Ok(code)
    }

    /// Unless the traffic doors are to be forced, puts them in a zone of
    /// their own that is in open mode at all times, and waits for every one
    /// to be in open mode.
    fn open_mode(&self, created: &mut Created) -> Result<(), String> {
        if !self.opens_in_open_mode() {
            return Ok(());
        }
        let name = format!("swarm-open-{}", random_hex(6));
        let doors = &created.doors[self.typing.len()..];
        let zone = json!({"name": name, "door_ids": doors, "open_schedule_ids": [ALWAYS_SCHEDULE]});
        created.open_zone = Some(self.create("/zones", &zone)?);
        self.wait_for("every traffic door to be in open mode", PATIENCE, || {
            Ok(self.traffic.iter().all(|door| door.record().open_mode()))
        })
    }

    /// The bytes each kind of door moved so far.
    fn bytes(&self) -> (Bytes, Bytes) {
        (bytes(&self.typing), bytes(&self.traffic))
    }

    /// What each kind of door moved since `before`, counted at `start`.
    fn since(&self, before: (Bytes, Bytes), start: Instant) -> Window {
        let (typing, traffic) = self.bytes();
        let window = start.elapsed();
        Window {
            typing: Moved {
                bytes: before.0.until(typing),
                window,
            },
            traffic: Moved {
                bytes: before.1.until(traffic),
                window,
            },
        }
    }

    /// The bytes moved while the doors sat idle, and for how long.
    fn idle(&self) -> Result<Window, String> {
        let before = self.bytes();
        let start = Instant::now();
        pause_until(start + self.options.idle, &self.interrupted)?;
        Ok(self.since(before, start))
    }

    /// Types the code and the submit key at every typing door, opens and
    /// closes every traffic door, and in a storm drops every connection
    /// halfway through.
    fn type_code(&self, code: &str) -> Result<Typed, String> {
        let before = self.bytes();
        let start = Instant::now();
        let end = start + self.options.duration;
        let typists = self.typists(&format!("{code}#"), end)?;
        let openers = self.openers(end)?;
        let cut = self.storm(start + self.options.duration / 2)?;
        let entries = joined(typists);
        let openings = joined(openers);
        pause_until(end, &self.interrupted)?;
        Ok(Typed {
            entries,
            openings,
            moved: self.since(before, start),
            cut,
        })
    }

    fn typists(&self, keys: &str, end: Instant) -> Result<Vec<thread::JoinHandle<u64>>, String> {
        let interval = Duration::from_secs(60) / self.options.rate;
        let (keys, interrupted) = (keys.to_owned(), Arc::clone(&self.interrupted));
        let work = move |door: &Door| {
            repeat(interval, end, &interrupted, || {
                door.press(&keys);
                Ok(())
            })
        };
        workers(&self.typing, "typist", work)
    }

    fn openers(&self, end: Instant) -> Result<Vec<thread::JoinHandle<u64>>, String> {
        let interval = Duration::from_secs(60) / self.options.traffic_rate;
        let (open_for, interrupted) = (self.options.open_for, Arc::clone(&self.interrupted));
        let work = move |door: &Door| {
            repeat(interval, end, &interrupted, || {
                open_once(door, open_for, &interrupted)
            })
        };
        workers(&self.traffic, "opener", work)
    }

    /// In a storm, drops every door's connection at `at` and returns when.
    fn storm(&self, at: Instant) -> Result<Option<Instant>, String> {
        if !self.options.storm {
            return Ok(None);
        }
        pause_until(at, &self.interrupted)?;
        let cut = Instant::now();
        for door in self.all() {
            door.cut();
        }
        Ok(Some(cut))
    }

    /// What the traffic doors did and the door events the server recorded
    /// for the run's doors; `None` without traffic doors.
    fn opened(
        &self,
        idle: Moved,
        typed: &Typed,
        created: &Created,
    ) -> Result<Option<Opened>, String> {
        if self.traffic.is_empty() {
            return Ok(None);
        }
        Ok(Some(Opened {
            openings_sent: typed.openings,
            recorded: self.recorded(&created.doors)?,
            idle,
            loaded: typed.moved.traffic,
        }))
    }

    /// The door events the server recorded for `doors`, added up.
    fn recorded(&self, doors: &[u64]) -> Result<DoorEvents, String> {
        let mut total = DoorEvents::default();
        for &door in doors {
            total = total.and(self.recorded_at(door)?);
        }
        Ok(total)
    }

    fn recorded_at(&self, door: u64) -> Result<DoorEvents, String> {
        Ok(DoorEvents {
            closed: self.events(door, "door", "closed")?,
            forced_open: self.events(door, "alarm", "forced%20open")?,
            held_open: self.events(door, "alarm", "held%20open")?,
        })
    }

    /// How many events of `kind` for `reason` the server recorded at `door`.
    fn events(&self, door: u64, kind: &str, reason: &str) -> Result<u64, String> {
        let query = format!("/events?door={door}&type={kind}&reason={reason}&per_page=1");
        let answer = self.server.get(&query)?;
        let total = answer["total"].as_u64();
        total.ok_or_else(|| format!("the server answered no total: {answer}"))
    }

    fn summary(&self, idle: Moved, typed: &Typed, traffic: Option<Opened>) -> Summary {
        let measured = Measured {
            duration: self.options.duration,
            entries_sent: typed.entries,
            idle,
            loaded: typed.moved.typing,
            cut: typed.cut,
            traffic,
        };
        Summary::of(&measured, &records(&self.typing), &records(&self.traffic))
    }
}

/// Runs `work` at each of `doors` on a thread of its own named for `kind`.
fn workers(
    doors: &[Arc<Door>],
    kind: &str,
    work: impl Fn(&Door) -> u64 + Clone + Send + 'static,
) -> Result<Vec<thread::JoinHandle<u64>>, String> {
    let spawn = |door: &Arc<Door>| {
        let (door, work) = (Arc::clone(door), work.clone());
        thread::Builder::new()
            .name(format!("{kind}-{}", door.hardware_id))
            .spawn(move || work(&door))
            .map_err(|error| error.to_string())
    };
    doors.iter().map(spawn).collect()
}
