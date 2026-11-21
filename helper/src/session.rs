//! One authenticated conversation with the strap over the Zepp channel:
//! handshake, live steps, and periodic sleep / minute-data downloads.
//!
//! Pure state: callers feed frames, bulk packets and clock ticks in, and get
//! back frames to write and JSON events to print.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use chrono::{DateTime, FixedOffset, Local, TimeZone};
use serde_json::{Value, json};

use crate::curve::{self, Curve, Gf};
use crate::frame::{Receiver, Sender};
use crate::proto::{self, FetchReply, Verdict};

const SLEEP_EVERY: Duration = Duration::from_secs(3600);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(180);
const SLEEP_DAYS_BACK: i64 = 8;
const BEFORE_ONSET: i64 = 3 * 3600;

pub enum Out {
    /// frame for the strap (characteristic 0016)
    Write(Vec<u8>),
    /// ack for a received frame (characteristic 0017)
    Ack(Vec<u8>),
    Event(Value),
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Phase {
    Authenticating,
    Ready,
    KeyRefused,
    Unsupported,
    Failed,
}

#[derive(PartialEq)]
enum What {
    Sleep,
    Minutes,
}

struct Transfer {
    what: What,
    began: Instant,
    from: Option<i64>,
    data: proto::Download,
}

pub struct Session {
    pub auth_key: [u8; 16],
    pub phase: Phase,
    pub began: Instant,
    mtu: usize,
    curve: Curve,
    private: Gf,
    tx: Sender,
    rx: Receiver,
    steps_encrypted: bool,
    fetch_encrypted: Option<bool>,
    can_fetch: bool,
    transfer: Option<Transfer>,
    last_sleep: Option<Instant>,
    measured: HashSet<i64>,
    waiting_onsets: Vec<i64>,
    day_start: Option<DateTime<FixedOffset>>,
}

fn status(state: &str) -> Out {
    Out::Event(json!({"t": "steps-status", "state": state}))
}

impl Session {
    /// `can_fetch`: the strap exposes the bulk-data characteristic (0005).
    pub fn begin(auth_key: [u8; 16], mtu: usize, can_fetch: bool, now: Instant) -> (Session, Vec<Out>) {
        let curve = curve::b163();
        let (private, public) = curve::keypair(&curve);
        let mut s = Session {
            auth_key,
            phase: Phase::Authenticating,
            began: now,
            mtu,
            curve,
            private,
            tx: Sender::default(),
            rx: Receiver::default(),
            steps_encrypted: false,
            fetch_encrypted: None,
            can_fetch,
            transfer: None,
            last_sleep: None,
            measured: HashSet::new(),
            waiting_onsets: Vec::new(),
            day_start: None,
        };
        let mut out = vec![status("authenticating")];
        s.send(&mut out, proto::EP_AUTH, &proto::hello(&public), false);
        (s, out)
    }

    fn send(&mut self, out: &mut Vec<Out>, endpoint: u16, payload: &[u8], encrypt: bool) {
        crate::debug(&format!("-> {endpoint:04x} {}", hex(payload)));
        out.extend(self.tx.frames(endpoint, payload, encrypt, self.mtu).into_iter().map(Out::Write));
    }

    fn stop(&mut self, out: &mut Vec<Out>, phase: Phase, state: &str) {
        self.phase = phase;
        out.push(status(state));
    }

    pub fn fail(&mut self) -> Vec<Out> {
        let mut out = Vec::new();
        self.stop(&mut out, Phase::Failed, "error");
        out
    }

    pub fn on_frame(&mut self, frame: &[u8], now: Instant) -> Vec<Out> {
        let mut out = Vec::new();
        let (message, ack) = self.rx.push(frame);
        if let Some(a) = ack {
            out.push(Out::Ack(a));
        }
        if let Some((endpoint, payload)) = message {
            crate::debug(&format!("<- {endpoint:04x} {}", hex(&payload)));
            self.on_message(&mut out, endpoint, &payload, now);
        }
        out
    }

    fn on_message(&mut self, out: &mut Vec<Out>, endpoint: u16, payload: &[u8], now: Instant) {
        match endpoint {
            proto::EP_AUTH => self.on_auth(out, payload, now),
            proto::EP_SERVICES if self.phase == Phase::Ready => {
                let Some(list) = proto::services(payload) else { return };
                let Some(&(_, steps_enc)) = list.iter().find(|(e, _)| *e == proto::EP_STEPS) else {
                    self.stop(out, Phase::Unsupported, "unsupported");
                    return;
                };
                self.steps_encrypted = steps_enc;
                if self.can_fetch {
                    self.fetch_encrypted = list.iter().find(|(e, _)| *e == proto::EP_FETCH).map(|(_, enc)| *enc);
                }
                self.send(out, proto::EP_STEPS, proto::STEPS_LIVE_ON, steps_enc);
                self.send(out, proto::EP_STEPS, proto::STEPS_TODAY, steps_enc);
                self.maybe_fetch_sleep(out, now, Local::now().fixed_offset());
            }
            proto::EP_STEPS if self.phase == Phase::Ready => {
                if let Some(s) = proto::steps(payload) {
                    let mut e = json!({"t": "steps", "steps": s.steps});
                    if let Some(d) = s.distance_m {
                        e["distance"] = d.into();
                    }
                    if let Some(k) = s.kcal {
                        e["calories"] = k.into();
                    }
                    out.push(Out::Event(e));
                }
            }
            proto::EP_FETCH if self.phase == Phase::Ready => self.on_fetch_reply(out, payload, now),
            _ => {}
        }
    }

    fn on_auth(&mut self, out: &mut Vec<Out>, payload: &[u8], _now: Instant) {
        if self.phase != Phase::Authenticating {
            return;
        }
        if let Some((random, point)) = proto::challenge(payload) {
            let Some(shared) = curve::shared(&self.curve, self.private, point) else {
                crate::debug("strap sent an invalid public point");
                self.stop(out, Phase::Failed, "error");
                return;
            };
            let session = proto::session(&self.auth_key, &shared);
            self.tx.session = Some(session.key);
            self.tx.sequence = session.first_sequence;
            self.rx.session = Some(session.key);
            let proof = proto::proof(&self.auth_key, &session.key, &random);
            self.send(out, proto::EP_AUTH, &proof, false);
            return;
        }
        match proto::verdict(payload) {
            Some(Verdict::Accepted) => {
                self.stop(out, Phase::Ready, "ok");
                self.send(out, proto::EP_SERVICES, proto::SERVICES_REQUEST, false);
            }
            Some(Verdict::KeyRefused) => self.stop(out, Phase::KeyRefused, "bad-key"),
            Some(Verdict::Failed) => self.stop(out, Phase::Failed, "error"),
            None => {}
        }
    }

    /// Every minute: ask for today's steps, and refresh sleep once an hour.
    pub fn tick(&mut self, now: Instant) -> Vec<Out> {
        let mut out = Vec::new();
        if self.phase == Phase::Ready {
            self.send(&mut out, proto::EP_STEPS, proto::STEPS_TODAY, self.steps_encrypted);
            self.maybe_fetch_sleep(&mut out, now, Local::now().fixed_offset());
        }
        out
    }

    fn maybe_fetch_sleep(&mut self, out: &mut Vec<Out>, now: Instant, local: DateTime<FixedOffset>) {
        if self.fetch_encrypted.is_none() {
            return;
        }
        if let Some(t) = &self.transfer {
            if now.duration_since(t.began) < DOWNLOAD_TIMEOUT {
                return;
            }
            crate::debug("download timed out");
            self.transfer = None;
        }
        if self.last_sleep.is_some_and(|t| now.duration_since(t) < SLEEP_EVERY) {
            return;
        }
        let since = local_noon(local - chrono::Duration::days(SLEEP_DAYS_BACK));
        self.download(out, What::Sleep, proto::KIND_SLEEP, &since, now);
    }

    fn download(&mut self, out: &mut Vec<Out>, what: What, kind: u8, since: &DateTime<FixedOffset>, now: Instant) {
        self.transfer = Some(Transfer { what, began: now, from: None, data: proto::Download::new() });
        let enc = self.fetch_encrypted.unwrap_or(false);
        self.send(out, proto::EP_FETCH, &proto::fetch_start(kind, since), enc);
    }

    pub fn on_bulk(&mut self, packet: &[u8]) {
        if let Some(t) = self.transfer.as_mut() {
            t.data.push(packet);
        }
    }

    fn on_fetch_reply(&mut self, out: &mut Vec<Out>, payload: &[u8], now: Instant) {
        let Some(reply) = proto::fetch_reply(payload) else { return };
        let enc = self.fetch_encrypted.unwrap_or(false);
        let Some(t) = self.transfer.as_mut() else { return };
        match reply {
            FetchReply::Started { count, from } => {
                t.from = from;
                if count == 0 {
                    self.finish(out, true, now);
                } else {
                    self.send(out, proto::EP_FETCH, proto::FETCH_SEND, enc);
                }
            }
            FetchReply::Sent { crc } => {
                let ok = t.data.intact(crc);
                self.finish(out, ok, now);
            }
            FetchReply::Refused { command, status } => {
                crate::debug(&format!("download refused: command {command:02x} status {status:02x}"));
                self.transfer = None;
                self.last_sleep = Some(now);
            }
            FetchReply::Acked => {}
        }
    }

    fn finish(&mut self, out: &mut Vec<Out>, ok: bool, now: Instant) {
        let Some(t) = self.transfer.take() else { return };
        let enc = self.fetch_encrypted.unwrap_or(false);
        self.send(out, proto::EP_FETCH, proto::FETCH_DONE_KEEP, enc);
        match t.what {
            What::Sleep => {
                self.last_sleep = Some(now);
                if ok {
                    self.after_sleep(out, &t.data.data, now, Local::now().fixed_offset());
                }
            }
            What::Minutes => {
                if let (true, Some(from), Some(day)) = (ok, t.from, self.day_start) {
                    let found = proto::latencies(from, &t.data.data, &self.waiting_onsets);
                    self.measured.extend(found.iter().map(|l| l.onset));
                    if !found.is_empty() {
                        let list: Vec<Value> = found
                            .iter()
                            .map(|l| json!({"start": l.onset, "latency": l.minutes, "bounded": l.at_least}))
                            .collect();
                        out.push(Out::Event(json!({"t": "latency", "nights": list})));
                    }
                    let hours = proto::steps_per_hour(from, &t.data.data, day.timestamp());
                    out.push(Out::Event(json!({"t": "hourly", "day": day.format("%Y-%m-%d").to_string(), "hours": hours})));
                }
                self.waiting_onsets.clear();
            }
        }
    }

    /// Report the nights, then fetch minute data from midnight (steps per hour)
    /// or from before any night not measured yet (time to fall asleep).
    fn after_sleep(&mut self, out: &mut Vec<Out>, data: &[u8], now: Instant, local: DateTime<FixedOffset>) {
        let all = proto::nights(data);
        let recent = &all[all.len().saturating_sub(14)..];
        let list: Vec<Value> = recent
            .iter()
            .map(|n| {
                json!({"start": n.start, "end": n.end, "asleep": n.asleep(), "deep": n.deep,
                       "light": n.light, "rem": n.rem, "awake": n.awake, "score": n.score})
            })
            .collect();
        out.push(Out::Event(json!({"t": "sleep", "nights": list})));

        let now_epoch = local.timestamp();
        self.waiting_onsets = recent
            .iter()
            .filter(|n| n.end <= now_epoch && !self.measured.contains(&n.start))
            .map(|n| n.start)
            .collect();
        let midnight = local_midnight(local);
        self.day_start = Some(midnight);
        let mut since = midnight;
        if let Some(&first) = self.waiting_onsets.iter().min() {
            let before = local.offset().timestamp_opt(first - BEFORE_ONSET, 0).single();
            if let Some(b) = before.filter(|b| *b < since) {
                since = b;
            }
        }
        self.download(out, What::Minutes, proto::KIND_MINUTES, &since, now);
    }
}

fn local_midnight(t: DateTime<FixedOffset>) -> DateTime<FixedOffset> {
    let date = t.date_naive().and_hms_opt(0, 0, 0).expect("midnight");
    Local.from_local_datetime(&date).earliest().map(|d| d.fixed_offset()).unwrap_or(t)
}

fn local_noon(t: DateTime<FixedOffset>) -> DateTime<FixedOffset> {
    let date = t.date_naive().and_hms_opt(12, 0, 0).expect("noon");
    Local.from_local_datetime(&date).earliest().map(|d| d.fixed_offset()).unwrap_or(t)
}

pub fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pretend strap: answers the handshake with a real key exchange.
    struct Strap {
        curve: Curve,
        key: [u8; 16],
        rx: Receiver,
        tx: Sender,
        random: [u8; 16],
        private: Gf,
        public: [u8; 48],
    }

    impl Strap {
        fn new(key: [u8; 16]) -> Strap {
            let curve = curve::b163();
            let (private, public) = curve::keypair(&curve);
            Strap { curve, key, rx: Receiver::default(), tx: Sender::default(), random: [0x5a; 16], private, public }
        }

        /// Messages the phone sent, decoded.
        fn read(&mut self, out: &[Out]) -> Vec<(u16, Vec<u8>)> {
            out.iter()
                .filter_map(|o| if let Out::Write(f) = o { self.rx.push(f).0 } else { None })
                .collect()
        }

        fn say(&mut self, endpoint: u16, payload: &[u8], encrypt: bool) -> Vec<Vec<u8>> {
            self.tx.frames(endpoint, payload, encrypt, 247)
        }
    }

    fn events(out: &[Out]) -> Vec<Value> {
        out.iter().filter_map(|o| if let Out::Event(e) = o { Some(e.clone()) } else { None }).collect()
    }

    fn feed(s: &mut Session, frames: Vec<Vec<u8>>, now: Instant) -> Vec<Out> {
        frames.iter().flat_map(|f| s.on_frame(f, now)).collect()
    }

    fn handshake(phone_key: [u8; 16], strap_key: [u8; 16]) -> (Session, Strap, Vec<Out>) {
        let now = Instant::now();
        let mut strap = Strap::new(strap_key);
        let (mut s, out) = Session::begin(phone_key, 247, false, now);
        let sent = strap.read(&out);
        assert_eq!(sent[0].0, proto::EP_AUTH);
        assert_eq!(sent[0].1[..4], [4, 2, 0, 2]);
        let phone_point = &sent[0].1[4..];

        let mut challenge = vec![0x10, 0x04, 0x01];
        challenge.extend(strap.random);
        challenge.extend(strap.public);
        let frames = strap.say(proto::EP_AUTH, &challenge, false);
        let out = feed(&mut s, frames, now);
        let proof = strap.read(&out).remove(0).1;

        let shared = curve::shared(&strap.curve, strap.private, phone_point).unwrap();
        let session = proto::session(&strap.key, &shared);
        let good = proof[1..17] == crate::frame::aes_block(&strap.key, &strap.random)
            && proof[17..33] == crate::frame::aes_block(&session.key, &strap.random);
        strap.rx.session = Some(session.key);
        strap.tx.session = Some(session.key);
        let frames = strap.say(proto::EP_AUTH, &[0x10, 0x05, if good { 0x01 } else { 0x25 }], false);
        let out = feed(&mut s, frames, now);
        (s, strap, out)
    }

    #[test]
    fn handshake_then_steps() {
        let key = [0x11; 16];
        let (mut s, mut strap, out) = handshake(key, key);
        assert_eq!(s.phase, Phase::Ready);
        assert_eq!(events(&out)[0]["state"], "ok");
        assert_eq!(strap.read(&out), vec![(proto::EP_SERVICES, vec![0x03])]);

        // services: steps is encrypted; no bulk characteristic, so no fetch
        let frames = strap.say(proto::EP_SERVICES, &[4, 1, 0, 0x16, 0, 1], false);
        let out = feed(&mut s, frames, Instant::now());
        let asked = strap.read(&out);
        assert_eq!(asked, vec![(proto::EP_STEPS, vec![5, 1]), (proto::EP_STEPS, vec![3])]);

        let mut live = vec![0x07, 0];
        live.extend(1234u32.to_le_bytes());
        live.extend(900u32.to_le_bytes());
        live.extend(55u32.to_le_bytes());
        let frames = strap.say(proto::EP_STEPS, &live, true);
        let e = events(&feed(&mut s, frames, Instant::now()));
        assert_eq!(e, vec![json!({"t": "steps", "steps": 1234, "distance": 900, "calories": 55})]);
    }

    #[test]
    fn wrong_key_is_reported() {
        let (s, _, out) = handshake([0x11; 16], [0x22; 16]);
        assert_eq!(s.phase, Phase::KeyRefused);
        assert_eq!(events(&out)[0]["state"], "bad-key");
    }

    #[test]
    fn missing_steps_endpoint() {
        let key = [0x11; 16];
        let (mut s, mut strap, _) = handshake(key, key);
        let frames = strap.say(proto::EP_SERVICES, &[4, 1, 0, 0x99, 0, 1], false);
        let out = feed(&mut s, frames, Instant::now());
        assert_eq!(s.phase, Phase::Unsupported);
        assert_eq!(events(&out)[0]["state"], "unsupported");
    }
}
