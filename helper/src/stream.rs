//! `helio-strap stream`: follow the strap over BlueZ and print one JSON object
//! per line for the panel.
//!
//! {"t":"status","state":"searching|disconnected|connecting|connected","name":…,"address":…}
//! {"t":"hr","bpm":72,"contact":true|false|null,"rr":[812,830]}
//! {"t":"battery","level":66}   {"t":"info","firmware":"3.27.0.1"}
//! {"t":"steps-status","state":"authenticating|ok|no-key|bad-key|unsupported|error"}
//! {"t":"steps",…} {"t":"sleep",…} {"t":"latency",…} {"t":"hourly",…}
//! {"t":"minutes","from":<epoch s of the first minute>,"steps":[…per minute…]}

use std::collections::HashMap;
use std::io::Write;
use std::time::{Duration, Instant, SystemTime};

use bluer::gatt::WriteOp;
use bluer::gatt::remote::{Characteristic, CharacteristicWriteRequest};
use bluer::{Adapter, AdapterEvent, Device, DeviceEvent, DeviceProperty, Uuid};
use futures::{Stream, StreamExt};
use serde_json::{Value, json};
use tokio::sync::mpsc;
use tokio::time::{MissedTickBehavior, interval, sleep, timeout};

use crate::keyring;
use crate::session::{Out, Phase, Session};

const RETRY: Duration = Duration::from_secs(30);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const STEPS_EVERY: Duration = Duration::from_secs(60);
const KEY_CHECK_EVERY: Duration = Duration::from_secs(15);
const AUTH_TIMEOUT: Duration = Duration::from_secs(30);
const ERROR_PAUSE: Duration = Duration::from_secs(120);
/// A refusal can be a passing one (the strap busy with the phone): try the same key again after this.
const REFUSED_PAUSE: Duration = Duration::from_secs(120);
const MAX_NAME: usize = 64;

fn std_uuid(short: u16) -> Uuid {
    Uuid::from_u128(0x0000_0000_0000_1000_8000_0080_5f9b_34fb | ((short as u128) << 96))
}
fn zepp_uuid(short: u16) -> Uuid {
    Uuid::from_u128(0x0000_0000_0000_3512_2118_0009_af10_0700 | ((short as u128) << 96))
}

/// Prints events; repeats of the last status lines are dropped.
#[derive(Default)]
struct Printer {
    status: Option<Value>,
    steps_status: Option<String>,
    candidates: Option<Value>,
}

impl Printer {
    fn print(&self, v: &Value) {
        let mut stdout = std::io::stdout().lock();
        if writeln!(stdout, "{v}").and_then(|_| stdout.flush()).is_err() {
            std::process::exit(0); // the panel went away
        }
    }

    fn status(&mut self, state: &str, dev: Option<&Info>) {
        let mut v = json!({"t": "status", "state": state});
        if let Some(d) = dev {
            v["name"] = d.name.clone().into();
            v["address"] = d.address.clone().into();
        }
        if self.status.as_ref() != Some(&v) {
            self.print(&v);
            self.status = Some(v);
        }
    }

    /// Like `status`, but always written: a failed write ends a helper the
    /// panel no longer reads.
    fn probe(&mut self, state: &str) {
        let v = json!({"t": "status", "state": state});
        self.print(&v);
        self.status = Some(v);
    }

    /// Straps the user could confirm: names and addresses only, nothing read from them.
    fn candidates(&mut self, list: &[Info]) {
        let devices: Vec<Value> = list.iter().map(|i| json!({"name": i.name, "address": i.address})).collect();
        let v = json!({"t": "candidates", "devices": devices});
        if self.candidates.as_ref() != Some(&v) {
            self.print(&v);
            self.candidates = Some(v);
        }
    }

    fn event(&mut self, v: Value) {
        if v["t"] == "steps-status" {
            let state = v["state"].as_str().unwrap_or_default().to_owned();
            if self.steps_status.as_deref() == Some(&state) {
                return;
            }
            self.steps_status = Some(state);
        }
        self.print(&v);
    }
}

struct Info {
    name: String,
    address: String,
    paired: bool,
    connected: bool,
    resolved: bool,
    heart_rate: bool,
}

async fn info(dev: &Device) -> Info {
    let alias = dev.alias().await.ok().filter(|a| !a.is_empty());
    let name = match alias {
        Some(a) => a,
        None => dev.name().await.ok().flatten().unwrap_or_default(),
    };
    Info {
        name: name.chars().filter(|c| !c.is_control()).take(MAX_NAME).collect(),
        address: dev.address().to_string(),
        paired: dev.is_paired().await.unwrap_or(false),
        connected: dev.is_connected().await.unwrap_or(false),
        resolved: dev.is_services_resolved().await.unwrap_or(false),
        heart_rate: dev.uuids().await.ok().flatten().is_some_and(|u| u.contains(&std_uuid(0x180d))),
    }
}

const MAX_CANDIDATES: usize = 8;

/// Paired devices whose name contains the filter (any heart rate strap when
/// the filter is empty). Only listed for the user to confirm: never connected
/// to or read from, because on a shared computer a paired strap may be
/// someone else's.
async fn candidates(adapter: &Adapter, filter: &str) -> Vec<Info> {
    let mut out = Vec::new();
    for addr in adapter.device_addresses().await.unwrap_or_default() {
        let Ok(dev) = adapter.device(addr) else { continue };
        let i = info(&dev).await;
        let wanted = i.paired && if filter.is_empty() { i.heart_rate } else { i.name.to_lowercase().contains(filter) };
        if wanted && out.len() < MAX_CANDIDATES {
            out.push(i);
        }
    }
    out.sort_by(|a, b| a.address.cmp(&b.address));
    out
}

/// The strap the user confirmed, if it is still paired.
async fn confirmed(adapter: &Adapter, address: bluer::Address) -> Option<(Device, Info)> {
    let dev = adapter.device(address).ok()?;
    let i = info(&dev).await;
    i.paired.then_some((dev, i))
}

/// Wait until BlueZ reports a change for this device or the adapter, or `limit`.
async fn changes(adapter: &Adapter, dev: Option<&Device>, limit: Duration) {
    let adapter_events = adapter.events().await.ok();
    let device_events = match dev {
        Some(d) => d.events().await.ok(),
        None => None,
    };
    let a = async {
        match adapter_events {
            Some(mut s) => loop {
                if let Some(AdapterEvent::DeviceAdded(_) | AdapterEvent::DeviceRemoved(_)) = s.next().await {
                    break;
                }
            },
            None => futures::future::pending().await,
        }
    };
    let d = async {
        match device_events {
            Some(mut s) => {
                s.next().await;
            }
            None => futures::future::pending().await,
        }
    };
    tokio::select! {
        _ = a => {}
        _ = d => {}
        _ = sleep(limit) => {}
    }
}

/// `address`: the strap the user confirmed in the panel. Without one, only
/// candidates are listed and nothing is read from any device.
pub async fn run(filter: String, reconnect: bool, address: Option<bluer::Address>) -> i32 {
    let filter = filter.trim().to_lowercase();
    let mut out = Printer::default();
    let adapter = match bluer::Session::new().await {
        Ok(s) => s.default_adapter().await,
        Err(e) => Err(e),
    };
    let adapter = match adapter {
        Ok(a) => a,
        Err(e) => {
            eprintln!("BlueZ unavailable: {e}");
            out.status("searching", None);
            sleep(RETRY).await;
            return 1; // the panel restarts the helper
        }
    };
    let Some(address) = address else {
        loop {
            let list = candidates(&adapter, &filter).await;
            out.candidates(&list);
            out.status("unconfirmed", None);
            changes(&adapter, None, RETRY).await;
        }
    };
    // One helper per strap: two at once (the shell reloading the panel) run
    // two handshakes over the same link and the strap refuses both.
    let _lock = loop {
        match take_lock() {
            Lock::Held(f) => break Some(f),
            Lock::Unavailable => break None,
            Lock::Busy => {
                out.probe("searching"); // exits once the panel that started us is gone
                sleep(Duration::from_secs(1)).await;
            }
        }
    };
    let mut zepp = Zepp::default();
    loop {
        let Some((dev, i)) = confirmed(&adapter, address).await else {
            out.status("searching", None);
            changes(&adapter, None, RETRY).await;
            continue;
        };
        if i.connected && i.resolved {
            connected(&dev, &i, &mut out, &mut zepp, reconnect).await;
            continue;
        }
        if i.connected {
            out.status("connecting", Some(&i));
            changes(&adapter, Some(&dev), RETRY).await;
            continue;
        }
        out.status("disconnected", Some(&i));
        changes(&adapter, Some(&dev), RETRY).await;
        if reconnect && !dev.is_connected().await.unwrap_or(false) {
            let _ = timeout(CONNECT_TIMEOUT, dev.connect()).await; // out of range or asleep: try later
        }
    }
}

enum Lock {
    Held(std::fs::File),
    Busy,
    /// No runtime directory or no locking: go on without.
    Unavailable,
}

fn take_lock() -> Lock {
    let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR") else { return Lock::Unavailable };
    let path = std::path::Path::new(&dir).join("helio-strap-stream.lock");
    let Ok(f) = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(path) else {
        return Lock::Unavailable;
    };
    match f.try_lock() {
        Ok(()) => Lock::Held(f),
        Err(std::fs::TryLockError::WouldBlock) => Lock::Busy,
        Err(_) => Lock::Unavailable,
    }
}

/// Zepp state that outlives one connection.
#[derive(Default)]
struct Zepp {
    rejected: Option<Refusal>,
}

struct Refusal {
    key: [u8; 16],
    when: Instant,
    /// `keyring::saved_at()` at the time: a later save means setup ran again.
    saved: Option<SystemTime>,
}

impl Refusal {
    /// Still too soon to offer this key again?
    fn holds(&self, key: &[u8; 16], now: Instant, saved: Option<SystemTime>) -> bool {
        self.key == *key && now.duration_since(self.when) < REFUSED_PAUSE && saved == self.saved
    }
}

enum Ev {
    Heart(Vec<u8>),
    Battery(Vec<u8>),
    Frame(Vec<u8>),
    Bulk(Vec<u8>),
    Device(DeviceEvent),
}

fn forward<S>(stream: S, tx: mpsc::UnboundedSender<Ev>, wrap: fn(S::Item) -> Ev) -> tokio::task::JoinHandle<()>
where
    S: Stream + Send + 'static,
    S::Item: Send,
{
    tokio::spawn(async move {
        let mut stream = Box::pin(stream);
        while let Some(item) = stream.next().await {
            if tx.send(wrap(item)).is_err() {
                break;
            }
        }
    })
}

struct Link {
    chars: HashMap<Uuid, Characteristic>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    tx: mpsc::UnboundedSender<Ev>,
    zepp_subscribed: bool,
    session: Option<Session>,
}

impl Drop for Link {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort(); // dropping the streams stops the notifications
        }
    }
}

impl Link {
    async fn subscribe(&mut self, short: Uuid, wrap: fn(Vec<u8>) -> Ev) {
        let Some(c) = self.chars.get(&short) else { return };
        match c.notify().await {
            Ok(s) => self.tasks.push(forward(s, self.tx.clone(), wrap)),
            Err(e) => eprintln!("Could not subscribe to {short}: {e}"),
        }
    }

    async fn write(&self, short: u16, data: &[u8]) {
        let Some(c) = self.chars.get(&zepp_uuid(short)) else { return };
        let req = CharacteristicWriteRequest { op_type: WriteOp::Command, ..Default::default() };
        if let Err(e) = c.write_ext(data, &req).await {
            crate::debug(&format!("write failed: {e}"));
        }
    }

    async fn apply(&mut self, outs: Vec<Out>, out: &mut Printer, zepp: &mut Zepp) {
        for o in outs {
            match o {
                Out::Write(f) => self.write(0x0016, &f).await,
                Out::Ack(a) => self.write(0x0017, &a).await,
                Out::Event(v) => out.event(v),
            }
        }
        if let Some(s) = &self.session {
            if s.phase == Phase::KeyRefused && zepp.rejected.as_ref().is_none_or(|r| r.when < s.began) {
                zepp.rejected = Some(Refusal { key: s.auth_key, when: Instant::now(), saved: keyring::saved_at() });
            }
        }
    }

    /// Start a Zepp session when there is a usable key; retry failures only
    /// with a different key, or after a pause.
    async fn check_key(&mut self, address: &str, out: &mut Printer, zepp: &mut Zepp) {
        if !self.chars.contains_key(&zepp_uuid(0x0016)) || !self.chars.contains_key(&zepp_uuid(0x0017)) {
            return;
        }
        let now = Instant::now();
        if let Some(s) = self.session.as_mut() {
            let age = now.duration_since(s.began);
            match s.phase {
                Phase::Ready | Phase::Unsupported => return,
                Phase::Authenticating if age < AUTH_TIMEOUT => return,
                Phase::Authenticating => {
                    let f = s.fail();
                    self.apply(f, out, zepp).await;
                    return;
                }
                Phase::Failed if age < ERROR_PAUSE => return,
                Phase::Failed | Phase::KeyRefused => {}
            }
        }
        let addr = address.to_owned();
        let key = tokio::task::spawn_blocking(move || keyring::lookup(&addr)).await.ok().flatten();
        let Some(key) = key else {
            self.session = None;
            out.event(json!({"t": "steps-status", "state": "no-key"}));
            return;
        };
        if zepp.rejected.as_ref().is_some_and(|r| r.holds(&key, now, keyring::saved_at())) {
            out.event(json!({"t": "steps-status", "state": "bad-key"}));
            return;
        }
        if !self.zepp_subscribed {
            self.zepp_subscribed = true;
            self.subscribe(zepp_uuid(0x0017), Ev::Frame).await;
            self.subscribe(zepp_uuid(0x0005), Ev::Bulk).await;
        }
        let mtu = match self.chars.get(&zepp_uuid(0x0016)) {
            Some(c) => c.mtu().await.unwrap_or(23),
            None => 23,
        };
        let can_fetch = self.chars.contains_key(&zepp_uuid(0x0005));
        let (s, outs) = Session::begin(key, mtu, can_fetch, now);
        self.session = Some(s);
        self.apply(outs, out, zepp).await;
    }
}

async fn connected(dev: &Device, i: &Info, out: &mut Printer, zepp: &mut Zepp, reconnect: bool) {
    out.status("connected", Some(i));
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut link = Link { chars: HashMap::new(), tasks: Vec::new(), tx, zepp_subscribed: false, session: None };
    if let Ok(services) = dev.services().await {
        for s in services {
            for c in s.characteristics().await.unwrap_or_default() {
                if let Ok(u) = c.uuid().await {
                    link.chars.entry(u).or_insert(c);
                }
            }
        }
    }
    if let Ok(events) = dev.events().await {
        link.tasks.push(forward(events, link.tx.clone(), Ev::Device));
    }
    if let Some(c) = link.chars.get(&std_uuid(0x2a28)) {
        if let Ok(fw) = c.read().await {
            let text: String = String::from_utf8_lossy(&fw).trim_matches(['\0', ' ']).chars().take(32).collect();
            out.event(json!({"t": "info", "firmware": text}));
        }
    }
    if let Some(c) = link.chars.get(&std_uuid(0x2a19)) {
        if let Ok(level) = c.read().await {
            if let Some(l) = level.first() {
                out.event(json!({"t": "battery", "level": l}));
            }
        }
    }
    link.subscribe(std_uuid(0x2a19), Ev::Battery).await;
    link.subscribe(std_uuid(0x2a37), Ev::Heart).await;
    link.check_key(&i.address, out, zepp).await;

    let mut key_check = interval(KEY_CHECK_EVERY);
    let mut steps = interval(STEPS_EVERY);
    for t in [&mut key_check, &mut steps] {
        t.set_missed_tick_behavior(MissedTickBehavior::Delay);
        t.reset();
    }
    loop {
        tokio::select! {
            ev = rx.recv() => match ev {
                None => return,
                Some(Ev::Heart(d)) => if let Some(hr) = crate::proto::heart_rate(&d) {
                    out.event(json!({"t": "hr", "bpm": hr.bpm, "contact": hr.contact, "rr": hr.rr_ms}));
                },
                Some(Ev::Battery(d)) => if let Some(l) = d.first() {
                    out.event(json!({"t": "battery", "level": l}));
                },
                Some(Ev::Frame(f)) => if let Some(s) = link.session.as_mut() {
                    let outs = s.on_frame(&f, Instant::now());
                    link.apply(outs, out, zepp).await;
                },
                Some(Ev::Bulk(b)) => if let Some(s) = link.session.as_mut() {
                    s.on_bulk(&b);
                },
                Some(Ev::Device(DeviceEvent::PropertyChanged(p))) => match p {
                    DeviceProperty::Connected(false) | DeviceProperty::ServicesResolved(false) => return,
                    DeviceProperty::BatteryPercentage(l) => out.event(json!({"t": "battery", "level": l})),
                    _ => {}
                },
            },
            _ = key_check.tick() => link.check_key(&i.address, out, zepp).await,
            _ = steps.tick() => if let Some(s) = link.session.as_mut() {
                let outs = s.tick(Instant::now());
                link.apply(outs, out, zepp).await;
            },
        }
        // A stalled history service clears on a new connection; the outer loop reconnects.
        if reconnect && link.session.as_ref().is_some_and(|s| s.wants_reconnect) {
            let _ = dev.disconnect().await;
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusal_lifts_after_a_pause_or_a_new_save() {
        let t0 = Instant::now();
        let saved = Some(SystemTime::UNIX_EPOCH);
        let r = Refusal { key: [1; 16], when: t0, saved };
        assert!(r.holds(&[1; 16], t0 + Duration::from_secs(60), saved));
        assert!(!r.holds(&[2; 16], t0, saved), "a different key is tried at once");
        assert!(!r.holds(&[1; 16], t0 + REFUSED_PAUSE, saved), "the same key is retried later");
        let resaved = Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1));
        assert!(!r.holds(&[1; 16], t0, resaved), "setup saving it again lifts the refusal");
    }

    #[test]
    fn uuids() {
        assert_eq!(std_uuid(0x2a37).to_string(), "00002a37-0000-1000-8000-00805f9b34fb");
        assert_eq!(zepp_uuid(0x0016).to_string(), "00000016-0000-3512-2118-0009af100700");
    }
}
