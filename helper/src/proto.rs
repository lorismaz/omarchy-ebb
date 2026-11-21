//! Message formats (PROTOCOL.md §1 and §2.2–2.7). Pure functions over bytes.

use chrono::{DateTime, FixedOffset, TimeZone};

pub const EP_SERVICES: u16 = 0x0000;
pub const EP_AUTH: u16 = 0x0082;
pub const EP_STEPS: u16 = 0x0016;
pub const EP_FETCH: u16 = 0x004b;

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}
fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

// ---- Heart rate (standard GATT 2a37) ----

#[derive(Debug, PartialEq)]
pub struct HeartRate {
    pub bpm: u16,
    pub contact: Option<bool>,
    pub rr_ms: Vec<u32>,
}

pub fn heart_rate(b: &[u8]) -> Option<HeartRate> {
    let flags = *b.first()?;
    let (bpm, mut at) = if flags & 0x01 != 0 { (u16_at(b, 1)?, 3) } else { (*b.get(1)? as u16, 2) };
    let contact = (flags & 0x04 != 0).then_some(flags & 0x02 != 0);
    if flags & 0x08 != 0 {
        at += 2;
    }
    let mut rr_ms = Vec::new();
    if flags & 0x10 != 0 {
        while let Some(v) = u16_at(b, at) {
            if rr_ms.len() == 16 {
                break;
            }
            rr_ms.push(((v as f64) * 1000.0 / 1024.0).round() as u32);
            at += 2;
        }
    }
    Some(HeartRate { bpm, contact, rr_ms })
}

// ---- Handshake ----

pub fn hello(public: &[u8; 48]) -> Vec<u8> {
    let mut m = vec![0x04, 0x02, 0x00, 0x02];
    m.extend_from_slice(public);
    m
}

/// `10 04 01 | random[16] | point[48]` -> (random, point)
pub fn challenge(b: &[u8]) -> Option<([u8; 16], &[u8])> {
    if b.len() < 67 || b[..3] != [0x10, 0x04, 0x01] {
        return None;
    }
    Some((b[3..19].try_into().ok()?, &b[19..67]))
}

pub struct Session {
    pub key: [u8; 16],
    pub first_sequence: u32,
}

pub fn session(auth_key: &[u8; 16], shared_point: &[u8; 48]) -> Session {
    Session {
        key: core::array::from_fn(|i| shared_point[8 + i] ^ auth_key[i]),
        first_sequence: u32_at(shared_point, 0).unwrap_or(0),
    }
}

pub fn proof(auth_key: &[u8; 16], session_key: &[u8; 16], random: &[u8; 16]) -> Vec<u8> {
    let mut m = vec![0x05];
    m.extend_from_slice(&crate::frame::aes_block(auth_key, random));
    m.extend_from_slice(&crate::frame::aes_block(session_key, random));
    m
}

#[derive(Debug, PartialEq)]
pub enum Verdict {
    Accepted,
    KeyRefused,
    Failed,
}

pub fn verdict(b: &[u8]) -> Option<Verdict> {
    match b {
        [0x10, 0x05, 0x01, ..] => Some(Verdict::Accepted),
        [0x10, 0x05, _, ..] => Some(Verdict::KeyRefused),
        [0x10, ..] => Some(Verdict::Failed),
        _ => None,
    }
}

// ---- Services ----

pub const SERVICES_REQUEST: &[u8] = &[0x03];

/// -> [(endpoint, encrypted)]
pub fn services(b: &[u8]) -> Option<Vec<(u16, bool)>> {
    if b.first() != Some(&0x04) {
        return None;
    }
    let count = u16_at(b, 1)? as usize;
    Some(
        b[3..]
            .chunks_exact(3)
            .take(count.min(512))
            .map(|e| (u16::from_le_bytes([e[0], e[1]]), e[2] != 0))
            .collect(),
    )
}

// ---- Steps ----

pub const STEPS_TODAY: &[u8] = &[0x03];
pub const STEPS_LIVE_ON: &[u8] = &[0x05, 0x01];

#[derive(Debug, PartialEq)]
pub struct Steps {
    pub steps: u32,
    pub distance_m: Option<u32>,
    pub kcal: Option<u32>,
}

pub fn steps(b: &[u8]) -> Option<Steps> {
    let block = match b {
        [0x07, rest @ ..] if rest.len() >= 13 => &rest[..13],
        [0x04, 0x01, rest @ ..] if rest.len() >= 13 => &rest[..13],
        _ => return None,
    };
    let (steps, distance, kcal) = (u32_at(block, 1)?, u32_at(block, 5)?, u32_at(block, 9)?);
    Some(Steps {
        steps,
        distance_m: (distance < 1_000_000).then_some(distance),
        kcal: (kcal < 100_000).then_some(kcal),
    })
}

// ---- History fetch ----

pub const KIND_SLEEP: u8 = 0x48;
pub const KIND_MINUTES: u8 = 0x01;
pub const FETCH_SEND: &[u8] = &[0x02];
pub const FETCH_DONE_KEEP: &[u8] = &[0x03, 0x09];

pub fn time8(t: &DateTime<FixedOffset>) -> [u8; 8] {
    use chrono::{Datelike, Timelike};
    let quarters = (t.offset().local_minus_utc() / 900) as i8;
    let y = (t.year() as u16).to_le_bytes();
    [y[0], y[1], t.month() as u8, t.day() as u8, t.hour() as u8, t.minute() as u8, 0, quarters as u8]
}

/// -> epoch seconds
pub fn read_time8(b: &[u8]) -> Option<i64> {
    let year = u16_at(b, 0)? as i32;
    let [month, day, hour, minute, second, tz] = b.get(2..8)?.try_into().ok()?;
    let offset = FixedOffset::east_opt(tz as i8 as i32 * 900)?;
    let t = offset.with_ymd_and_hms(year, month as u32, day as u32, hour as u32, minute as u32, second as u32);
    Some(t.single()?.timestamp())
}

pub fn fetch_start(kind: u8, since: &DateTime<FixedOffset>) -> Vec<u8> {
    let mut m = vec![0x01, kind];
    m.extend_from_slice(&time8(since));
    m
}

#[derive(Debug, PartialEq)]
pub enum FetchReply {
    Started { count: u32, from: Option<i64> },
    Sent { crc: Option<u32> },
    Acked,
    Refused { command: u8, status: u8 },
}

pub fn fetch_reply(b: &[u8]) -> Option<FetchReply> {
    let [0x10, command, status, ..] = *b else { return None };
    if status != 0x01 {
        return Some(FetchReply::Refused { command, status });
    }
    match command {
        0x01 => Some(FetchReply::Started { count: u32_at(b, 3)?, from: b.get(7..15).and_then(read_time8) }),
        0x02 => Some(FetchReply::Sent { crc: u32_at(b, 3) }),
        0x03 => Some(FetchReply::Acked),
        _ => None,
    }
}

/// Bulk history packets: `counter | data`, counters 0, 1, 2… with no gaps.
pub struct Download {
    pub data: Vec<u8>,
    expect: u8,
    pub broken: bool,
}

impl Download {
    const LIMIT: usize = 512 * 1024;

    pub fn new() -> Self {
        Download { data: Vec::new(), expect: 0, broken: false }
    }

    pub fn push(&mut self, packet: &[u8]) {
        let Some((&counter, rest)) = packet.split_first() else { return };
        if self.broken || counter != self.expect || self.data.len() + rest.len() > Self::LIMIT {
            self.broken = true;
            return;
        }
        self.expect = self.expect.wrapping_add(1);
        self.data.extend_from_slice(rest);
    }

    pub fn intact(&self, crc: Option<u32>) -> bool {
        !self.broken && crc.is_none_or(|c| crc32fast::hash(&self.data) == c)
    }
}

// ---- Sleep sessions (§2.6) ----

pub const SLEEP_RECORD: usize = 594;

#[derive(Debug, Clone, PartialEq)]
pub struct Night {
    pub written: u32,
    pub start: i64,
    pub end: i64,
    pub rem: u16,
    pub light: u16,
    pub deep: u16,
    pub awake: u16,
    pub score: u8,
}

impl Night {
    pub fn asleep(&self) -> u32 {
        self.rem as u32 + self.light as u32 + self.deep as u32
    }
}

fn night(r: &[u8]) -> Option<Night> {
    let written = u32_at(r, 0x00)?;
    let midnight = u32_at(r, 0x04)? as i64;
    let previous_midnight = midnight - 24 * 3600;
    let stages = (r[0x54] as usize).min((0x24A - 0x56) / 5);
    let (mut start, mut end) = (i64::MAX, i64::MIN);
    for s in r[0x56..].chunks_exact(5).take(stages) {
        let (from, to) = (u16::from_le_bytes([s[0], s[1]]) as i64, u16::from_le_bytes([s[2], s[3]]) as i64);
        if to < from || to - from > 24 * 60 {
            continue;
        }
        start = start.min(previous_midnight + from * 60);
        end = end.max(previous_midnight + (to + 1) * 60);
    }
    (start < end).then(|| Night {
        written,
        start,
        end,
        rem: u16_at(r, 0x24A).unwrap_or(0),
        light: u16_at(r, 0x24C).unwrap_or(0),
        deep: u16_at(r, 0x24E).unwrap_or(0),
        awake: u16_at(r, 0x250).unwrap_or(0),
        score: r[0x16],
    })
}

/// Nights in time order; for overlapping reports the last-written one wins.
pub fn nights(data: &[u8]) -> Vec<Night> {
    let mut all: Vec<Night> = data.chunks_exact(SLEEP_RECORD).filter_map(night).collect();
    all.sort_by_key(|n| n.written);
    let mut kept: Vec<Night> = Vec::new();
    for n in all {
        kept.retain(|k| k.end <= n.start || k.start >= n.end);
        kept.push(n);
    }
    kept.sort_by_key(|n| n.start);
    kept
}

// ---- Minute samples (§2.7) ----

const MINUTE: usize = 8;
const NOT_WORN: u8 = 115;
const CHARGING: u8 = 118;
const STILL_UP_TO: u8 = 40;
const LONGEST_WAIT: u32 = 120;

#[derive(Debug, PartialEq)]
pub struct Latency {
    pub onset: i64,
    pub minutes: u32,
    /// The strap was put on (or the data starts) during the wait: a lower bound.
    pub at_least: bool,
}

/// How long before each sleep onset the wearer lay still.
pub fn latencies(first_minute: i64, data: &[u8], onsets: &[i64]) -> Vec<Latency> {
    let minutes: Vec<&[u8]> = data.chunks_exact(MINUTE).collect();
    let mut out = Vec::new();
    for &onset in onsets {
        let at = (onset - first_minute).div_euclid(60);
        if at <= 0 || at as usize > minutes.len() {
            continue;
        }
        let (mut still, mut at_least) = (0u32, false);
        let mut back = minutes[..at as usize].iter().rev();
        loop {
            let Some(m) = back.next() else {
                at_least = true;
                break;
            };
            if m[0] == NOT_WORN || m[0] == CHARGING {
                at_least = true;
                break;
            }
            if m[2] > 0 || m[1] > STILL_UP_TO {
                break;
            }
            still += 1;
            if still >= LONGEST_WAIT {
                at_least = true;
                break;
            }
        }
        out.push(Latency { onset, minutes: still, at_least });
    }
    out
}

pub fn steps_per_hour(first_minute: i64, data: &[u8], day_start: i64) -> [u32; 24] {
    let mut hours = [0u32; 24];
    for (i, m) in data.chunks_exact(MINUTE).enumerate() {
        let since = first_minute + 60 * i as i64 - day_start;
        if (0..86_400).contains(&since) {
            hours[(since / 3600) as usize] += m[2] as u32;
        }
    }
    hours
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heart_rate_packets() {
        assert_eq!(heart_rate(&[0x00, 72]), Some(HeartRate { bpm: 72, contact: None, rr_ms: vec![] }));
        assert_eq!(heart_rate(&[0x01, 0x2c, 0x01]).unwrap().bpm, 300);
        assert_eq!(heart_rate(&[0x06, 80]).unwrap().contact, Some(true));
        assert_eq!(heart_rate(&[0x04, 0]).unwrap().contact, Some(false));
        assert_eq!(heart_rate(&[0x10, 70, 0x00, 0x04]).unwrap().rr_ms, vec![1000]);
        assert_eq!(heart_rate(&[0x18, 70, 1, 2, 0x00, 0x04]).unwrap().rr_ms, vec![1000]);
        assert_eq!(heart_rate(&[]), None);
        assert_eq!(heart_rate(&[1, 5]), None);
        let many: Vec<u8> = [0x10, 70].into_iter().chain([0, 4].repeat(40)).collect();
        assert_eq!(heart_rate(&many).unwrap().rr_ms.len(), 16);
    }

    #[test]
    fn handshake_messages() {
        let mut c = vec![0x10, 0x04, 0x01];
        c.extend([b'r'; 16]);
        c.extend([7u8; 48]);
        let (random, point) = challenge(&c).unwrap();
        assert_eq!((random, point), ([b'r'; 16], &[7u8; 48][..]));
        assert!(challenge(&[0x10, 0x04, 0x02].iter().copied().chain([0; 64]).collect::<Vec<_>>()).is_none());
        assert_eq!(verdict(&[0x10, 0x05, 0x01]), Some(Verdict::Accepted));
        assert_eq!(verdict(&[0x10, 0x05, 0x25]), Some(Verdict::KeyRefused));
        assert_eq!(verdict(&[0x10, 0x04, 0x02]), Some(Verdict::Failed));
        assert_eq!(proof(&[1; 16], &[2; 16], &[3; 16]).len(), 33);
        let s = session(&[0xff; 16], &core::array::from_fn(|i| i as u8));
        assert_eq!(s.first_sequence, u32::from_le_bytes([0, 1, 2, 3]));
        assert_eq!(s.key[0], 8 ^ 0xff);
    }

    #[test]
    fn services_and_steps() {
        assert_eq!(services(&[4, 2, 0, 0x16, 0, 1, 0x82, 0, 0]), Some(vec![(0x16, true), (0x82, false)]));
        assert_eq!(services(&[4, 0xff, 0xff]), Some(vec![]));
        let mut live = vec![0x07, 0x0c];
        live.extend(4321u32.to_le_bytes());
        live.extend(3100u32.to_le_bytes());
        live.extend(150u32.to_le_bytes());
        assert_eq!(steps(&live), Some(Steps { steps: 4321, distance_m: Some(3100), kcal: Some(150) }));
        let mut reply = vec![0x04, 0x01, 0x0c];
        reply.extend(5u32.to_le_bytes());
        reply.extend([0xff; 8]);
        assert_eq!(steps(&reply), Some(Steps { steps: 5, distance_m: None, kcal: None }));
        assert_eq!(steps(&[0x04, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]), None);
        assert_eq!(steps(&[0x07]), None);
    }

    #[test]
    fn fetch_messages() {
        let plus1 = FixedOffset::east_opt(3600).unwrap();
        let since = plus1.with_ymd_and_hms(2026, 11, 19, 20, 0, 0).unwrap();
        let hex: String = fetch_start(KIND_SLEEP, &since).iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "0148ea070b1314000004");
        let start = [0x10, 0x01, 0x01, 0x03, 0x06, 0, 0, 0xea, 0x07, 0x0b, 0x13, 0x16, 0, 0, 0x04, 0];
        let from = plus1.with_ymd_and_hms(2026, 11, 19, 22, 0, 0).unwrap().timestamp();
        assert_eq!(fetch_reply(&start), Some(FetchReply::Started { count: 0x603, from: Some(from) }));
        assert_eq!(read_time8(&[0; 8]), None);
        assert_eq!(fetch_reply(&[0x10, 0x02, 0x01, 7, 0, 0, 0]), Some(FetchReply::Sent { crc: Some(7) }));
        assert_eq!(fetch_reply(&[0x10, 0x02, 0x01]), Some(FetchReply::Sent { crc: None }));
        assert_eq!(fetch_reply(&[0x10, 0x03, 0x01]), Some(FetchReply::Acked));
        assert_eq!(fetch_reply(&[0x10, 0x01, 0x02]), Some(FetchReply::Refused { command: 1, status: 2 }));
        assert_eq!(fetch_reply(&[0x04, 0x01]), None);
        let mut d = Download::new();
        d.push(b"\x00ab");
        d.push(b"\x01cd");
        assert_eq!(d.data, b"abcd");
        assert!(d.intact(Some(crc32fast::hash(b"abcd"))) && !d.intact(Some(1)));
        d.push(b"\x05ef");
        assert!(d.broken);
    }

    fn record(written: u32, midnight: u32, stages: &[(u16, u16, u8)], totals: [u16; 4]) -> Vec<u8> {
        let mut r = vec![0u8; SLEEP_RECORD];
        r[0..4].copy_from_slice(&written.to_le_bytes());
        r[4..8].copy_from_slice(&midnight.to_le_bytes());
        r[0x15] = 58;
        r[0x16] = 81;
        r[0x54] = stages.len() as u8;
        for (i, (s, e, k)) in stages.iter().enumerate() {
            let at = 0x56 + 5 * i;
            r[at..at + 2].copy_from_slice(&s.to_le_bytes());
            r[at + 2..at + 4].copy_from_slice(&e.to_le_bytes());
            r[at + 4] = *k;
        }
        for (i, t) in totals.iter().enumerate() {
            r[0x24A + 2 * i..0x24C + 2 * i].copy_from_slice(&t.to_le_bytes());
        }
        r
    }

    #[test]
    fn sleep_records() {
        let mid: u32 = 1_795_215_600;
        let base = mid as i64 - 86_400;
        let a = record(mid + 30_000, mid, &[(1320, 1379, 4), (1380, 1679, 5), (1680, 1769, 8)], [90, 60, 300, 30]);
        let n = nights(&a);
        assert_eq!(n.len(), 1);
        assert_eq!((n[0].start, n[0].end), (base + 1320 * 60, base + 1770 * 60));
        assert_eq!((n[0].asleep(), n[0].awake, n[0].score), (450, 30, 81));
        let newer = record(mid + 40_000, mid, &[(1330, 1780, 4)], [0, 451, 0, 0]);
        let empty = record(mid, mid, &[], [0; 4]);
        let n = nights(&[a, newer, empty].concat());
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].asleep(), 451);
        assert!(nights(&[0; 100]).is_empty());
    }

    fn minute(kind: u8, intensity: u8, steps: u8) -> [u8; 8] {
        [kind, intensity, steps, 65, 5, 0, 0, 0]
    }
    fn repeat(m: [u8; 8], n: usize) -> Vec<u8> {
        m.repeat(n)
    }

    #[test]
    fn falling_asleep() {
        let t0 = 1_800_000_000;
        let data = [repeat(minute(112, 60, 12), 10), repeat(minute(112, 5, 0), 12), repeat(minute(120, 0, 0), 30)].concat();
        assert_eq!(latencies(t0, &data, &[t0 + 22 * 60]), vec![Latency { onset: t0 + 1320, minutes: 12, at_least: false }]);
        // strap put on just before bed: not worn, then still minutes (intensity 15 counts as still)
        let real = [repeat(minute(115, 0, 0), 40), repeat(minute(112, 15, 0), 1), repeat(minute(112, 0, 0), 8), repeat(minute(120, 0, 0), 10)].concat();
        assert_eq!(latencies(t0, &real, &[t0 + 49 * 60]), vec![Latency { onset: t0 + 2940, minutes: 9, at_least: true }]);
        let charging = [repeat(minute(118, 0, 0), 1), repeat(minute(112, 0, 0), 5), repeat(minute(120, 0, 0), 1)].concat();
        assert!(latencies(t0, &charging, &[t0 + 6 * 60])[0].at_least);
        let still = repeat(minute(112, 0, 0), 5);
        assert_eq!(latencies(t0, &still, &[t0 + 300]), vec![Latency { onset: t0 + 300, minutes: 5, at_least: true }]);
        let long = repeat(minute(112, 0, 0), 300);
        assert_eq!(latencies(t0, &long, &[t0 + 250 * 60])[0], Latency { onset: t0 + 15_000, minutes: 120, at_least: true });
        assert!(latencies(t0, &still, &[t0 - 60, t0 + 9999 * 60]).is_empty());
    }

    #[test]
    fn hourly() {
        let day = 1_800_000_000;
        let data = [repeat(minute(112, 0, 10), 60), repeat(minute(112, 0, 0), 60), repeat(minute(112, 0, 3), 30)].concat();
        let h = steps_per_hour(day - 30 * 60, &data, day);
        assert_eq!((h[0], h[1], h[2], h.iter().sum::<u32>()), (300, 90, 0, 390));
        assert_eq!(steps_per_hour(day, &[], day), [0; 24]);
    }
}
