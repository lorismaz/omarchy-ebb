.pragma library

// Pure helpers for the Helio Strap plugin, kept free of QML so
// tests/model.test.js can exercise them under node.

var MAX_LINE = 2048
var MAX_NAME = 64
var MIN_BPM = 25
var MAX_BPM = 240
var HISTORY_SECONDS = 600     // the chart spans the last ten minutes
var SAMPLE_SECONDS = 5        // one chart point every five seconds
var MAX_POINTS = HISTORY_SECONDS / SAMPLE_SECONDS
var STALE_MS = 15000          // no reading for this long: show "--"
var STATES = ["unconfirmed", "searching", "disconnected", "connecting", "connected"]

function clampInt(v, lo, hi) {
  var n = Math.round(Number(v))
  if (!isFinite(n) || n < lo || n > hi) return null
  return n
}

function cleanText(v, max) {
  var s = String(v === undefined || v === null ? "" : v).replace(/[\u0000-\u001f\u007f]/g, "")
  return s.length > max ? s.slice(0, max - 1) + "…" : s
}

// "1.0.0" < "1.1.0"; anything unparsable counts as older.
function versionLess(v, min) {
  var parse = function(x) { var m = /^(\d+)\.(\d+)\.(\d+)$/.exec(String(x || "").trim()); return m ? [+m[1], +m[2], +m[3]] : null }
  var a = parse(v), b = parse(min)
  if (!a || !b) return true
  for (var i = 0; i < 3; i++) if (a[i] !== b[i]) return a[i] < b[i]
  return false
}

// A Bluetooth address "AA:BB:CC:DD:EE:FF" (upper case), or "" if not one.
var MAX_CANDIDATES = 8
function normalizeAddress(v) {
  var s = String(v === undefined || v === null ? "" : v).trim().toUpperCase()
  return /^[0-9A-F]{2}(:[0-9A-F]{2}){5}$/.test(s) ? s : ""
}

// One line of `helio-strap stream` output -> a validated event, or null.
function parseLine(line) {
  var raw = String(line || "")
  if (raw.length === 0 || raw.length > MAX_LINE) return null
  var obj
  try { obj = JSON.parse(raw) } catch (e) { return null }
  if (!obj || typeof obj !== "object") return null

  if (obj.t === "hr") {
    var bpm = clampInt(obj.bpm, 0, 300)
    if (bpm === null) return null
    var rr = []
    if (Array.isArray(obj.rr)) {
      for (var i = 0; i < obj.rr.length && rr.length < 16; i++) {
        var r = clampInt(obj.rr[i], 200, 2500)
        if (r !== null) rr.push(r)
      }
    }
    var contact = obj.contact === true ? true : obj.contact === false ? false : null
    // A strap off the wrist reports 0 or a lost contact; neither is a reading.
    var valid = bpm >= MIN_BPM && bpm <= MAX_BPM && contact !== false
    return { type: "hr", bpm: bpm, valid: valid, contact: contact, rr: rr }
  }
  if (obj.t === "battery") {
    var level = clampInt(obj.level, 0, 100)
    return level === null ? null : { type: "battery", level: level }
  }
  if (obj.t === "candidates") {
    if (!Array.isArray(obj.devices)) return null
    var devices = []
    for (var i = 0; i < obj.devices.length && devices.length < MAX_CANDIDATES; i++) {
      var d = obj.devices[i]
      var address = d && typeof d === "object" ? normalizeAddress(d.address) : ""
      if (address !== "") devices.push({ name: cleanText(d.name, MAX_NAME), address: address })
    }
    return { type: "candidates", devices: devices }
  }
  if (obj.t === "status") {
    if (STATES.indexOf(obj.state) < 0) return null
    return { type: "status", state: obj.state, name: cleanText(obj.name, MAX_NAME) }
  }
  if (obj.t === "info") {
    return { type: "info", firmware: cleanText(obj.firmware, 32) }
  }
  if (obj.t === "steps") {
    var steps = clampInt(obj.steps, 0, 500000)
    if (steps === null) return null
    // Distance and calories are only sent when they look sane, and the
    // device's layout for them is unconfirmed, so both may be missing.
    var dist = obj.distance === undefined ? null : clampInt(obj.distance, 0, 1000000)
    var kcal = obj.calories === undefined ? null : clampInt(obj.calories, 0, 100000)
    return { type: "steps", steps: steps, distance: dist, calories: kcal }
  }
  if (obj.t === "sleep") {
    if (!Array.isArray(obj.nights)) return null
    var nights = []
    for (var n = 0; n < obj.nights.length && nights.length < 14; n++) {
      var night = parseNight(obj.nights[n])
      if (night) nights.push(night)
    }
    return { type: "sleep", nights: nights }
  }
  if (obj.t === "hourly") {
    if (!/^\d{4}-\d{2}-\d{2}$/.test(String(obj.day)) || !Array.isArray(obj.hours) || obj.hours.length !== 24) return null
    var hours = []
    for (var hh = 0; hh < 24; hh++) {
      var v = clampInt(obj.hours[hh], 0, 30000)
      hours.push(v === null ? 0 : v)
    }
    return { type: "hourly", day: obj.day, hours: hours }
  }
  if (obj.t === "latency") {
    if (!Array.isArray(obj.nights)) return null
    var list = []
    for (var l = 0; l < obj.nights.length && list.length < 31; l++) {
      var e = obj.nights[l]
      if (!e || typeof e !== "object") continue
      var st = Number(e.start), lat = clampInt(e.latency, 0, 120)
      if (!(st > 1577836800 && st < 4102444800) || lat === null) continue
      list.push({ start: st, latency: lat, bounded: e.bounded === true })
    }
    return { type: "latency", nights: list }
  }
  if (obj.t === "steps-status") {
    if (STEPS_STATES.indexOf(obj.state) < 0) return null
    return { type: "steps-status", state: obj.state }
  }
  return null
}

var STEPS_STATES = ["no-key", "authenticating", "ok", "bad-key", "error", "unsupported"]

function normalizeStepGoal(v) {
  var n = clampInt(v, 500, 100000)
  return n === null ? 8000 : n
}

// The bar setting is stored as the label shown in Omarchy's settings.
var BAR_MODES = { "Heart rate": "hr", "Steps": "steps", "Heart rate and steps": "both" }

function normalizeBarMode(v) {
  var s = String(v || "")
  if (BAR_MODES.hasOwnProperty(s)) return BAR_MODES[s]
  return s === "steps" || s === "hr" ? s : "both"
}

// 0..1, capped
function goalProgress(steps, goal) {
  if (!(steps >= 0) || !(goal > 0)) return 0
  return Math.min(1, steps / goal)
}

// Zones as a share of maximum heart rate (the Zepp/Polar five-zone model).
// 0 = below zone 1 (resting).
var ZONE_FLOORS = [0.5, 0.6, 0.7, 0.8, 0.9]

function zoneFor(bpm, maxHr) {
  var max = Number(maxHr) > 0 ? Number(maxHr) : 190
  var pct = Number(bpm) / max
  var z = 0
  for (var i = 0; i < ZONE_FLOORS.length; i++) if (pct >= ZONE_FLOORS[i]) z = i + 1
  return z
}

function zoneBounds(zone, maxHr) {
  var max = Number(maxHr) > 0 ? Number(maxHr) : 190
  if (zone <= 0) return [0, Math.round(max * ZONE_FLOORS[0]) - 1]
  var lo = Math.round(max * ZONE_FLOORS[zone - 1])
  var hi = zone < 5 ? Math.round(max * ZONE_FLOORS[zone]) - 1 : max
  return [lo, hi]
}

function normalizeMaxHr(v) {
  var n = clampInt(v, 100, 230)
  return n === null ? 190 : n
}

// Session statistics, rebuilt immutably so QML bindings notice the change.
function emptyStats() {
  return { count: 0, sum: 0, min: 0, max: 0, zoneSeconds: [0, 0, 0, 0, 0, 0], lastAt: 0 }
}

function addReading(stats, bpm, zone, atMs) {
  var s = {
    count: stats.count + 1,
    sum: stats.sum + bpm,
    min: stats.count === 0 ? bpm : Math.min(stats.min, bpm),
    max: stats.count === 0 ? bpm : Math.max(stats.max, bpm),
    zoneSeconds: stats.zoneSeconds.slice(),
    lastAt: atMs
  }
  // Credit the time since the previous reading to the current zone, but
  // never more than a few seconds, so a gap in the stream is not counted.
  if (stats.lastAt > 0) {
    var dt = Math.min(5, Math.max(0, (atMs - stats.lastAt) / 1000))
    s.zoneSeconds[zone] += dt
  }
  return s
}

function average(stats) {
  return stats.count > 0 ? Math.round(stats.sum / stats.count) : 0
}

function pushPoint(points, bpm) {
  var out = points.slice(points.length >= MAX_POINTS ? points.length - MAX_POINTS + 1 : 0)
  out.push(bpm)
  return out
}

// RMSSD over the most recent RR intervals (ms): a common short-term HRV figure.
function rmssd(rrs) {
  if (!rrs || rrs.length < 3) return 0
  var sum = 0
  for (var i = 1; i < rrs.length; i++) {
    var d = rrs[i] - rrs[i - 1]
    sum += d * d
  }
  return Math.round(Math.sqrt(sum / (rrs.length - 1)))
}

function pushRR(buffer, rr) {
  var out = buffer.concat(rr)
  return out.length > 60 ? out.slice(out.length - 60) : out
}

function pad2(n) { return n < 10 ? "0" + n : String(n) }

// 0:07, 4:32, 1:02:09
function formatDuration(totalSeconds) {
  var t = Math.max(0, Math.floor(Number(totalSeconds) || 0))
  var h = Math.floor(t / 3600)
  var m = Math.floor((t % 3600) / 60)
  var s = t % 60
  return h > 0 ? h + ":" + pad2(m) + ":" + pad2(s) : m + ":" + pad2(s)
}

function batteryIcon(level) {
  if (level === null || level === undefined || level < 0) return "󰂑"
  var icons = ["󰂎", "󰁺", "󰁻", "󰁼", "󰁽", "󰁾", "󰁿", "󰂀", "󰂁", "󰂂", "󰁹"]
  return icons[Math.min(10, Math.floor(level / 10))]
}

// ---------------------------------------------------------------------------
// Working late / screen time
// ---------------------------------------------------------------------------
var DAY_BOUNDARY_HOUR = 5     // a "day" runs 05:00 -> 05:00, so a late night stays with its evening
var HEADS_UP_MINUTES = 30
var LATE_WINDOW_MINUTES = 6 * 60

// "23:00" / "0:30" -> minutes after midnight; null if invalid.
function parseClock(value) {
  var m = /^\s*(\d{1,2})[:h.](\d{2})\s*$/.exec(String(value || ""))
  if (!m) return null
  var h = parseInt(m[1], 10), min = parseInt(m[2], 10)
  if (h > 23 || min > 59) return null
  return h * 60 + min
}

function normalizeBedtime(value) {
  var v = parseClock(value)
  return v === null ? 23 * 60 : v
}

function formatClock(minutes) {
  var m = ((Math.round(minutes) % 1440) + 1440) % 1440
  return pad2(Math.floor(m / 60)) + ":" + pad2(m % 60)
}

function minutesOfDay(date) {
  return date.getHours() * 60 + date.getMinutes() + date.getSeconds() / 60
}

// Local calendar key of the "day" a moment belongs to (shifted by the boundary).
function dayKey(date) {
  var d = new Date(date.getTime() - DAY_BOUNDARY_HOUR * 3600 * 1000)
  return d.getFullYear() + "-" + pad2(d.getMonth() + 1) + "-" + pad2(d.getDate())
}

// Signed minutes relative to bedtime, in (-720, 720]: negative before it.
function minutesPastBedtime(date, bedtime) {
  var delta = minutesOfDay(date) - bedtime
  if (delta > 720) delta -= 1440
  if (delta <= -720) delta += 1440
  return delta
}

// Minutes until tonight's bedtime. minutesPastBedtime wraps at +-12h, so in the
// morning it reads as hours past last night's bedtime; count to the next one instead.
function minutesUntilBedtime(pastBedtime) {
  return pastBedtime < 0 ? -pastBedtime : 1440 - pastBedtime
}

// Day record persisted to the state file; anything malformed becomes a fresh day.
function freshDay(key) {
  return { day: key, activeMinutes: 0, firstActiveAt: 0, headsUpDone: false,
           lastAlertAt: 0, snoozedUntil: 0, doneForTonight: false,
           windDownDone: false, bedtimeDone: false, lateCount: 0, urgentDone: false, morningDone: false }
}

function loadDay(text, key) {
  var obj
  try { obj = JSON.parse(String(text || "")) } catch (e) { return freshDay(key) }
  if (!obj || typeof obj !== "object" || obj.day !== key) return freshDay(key)
  var d = freshDay(key)
  d.activeMinutes = clampInt(obj.activeMinutes, 0, 1440) || 0
  d.firstActiveAt = Number(obj.firstActiveAt) > 0 ? Number(obj.firstActiveAt) : 0
  d.headsUpDone = obj.headsUpDone === true
  d.lastAlertAt = Number(obj.lastAlertAt) > 0 ? Number(obj.lastAlertAt) : 0
  d.snoozedUntil = Number(obj.snoozedUntil) > 0 ? Number(obj.snoozedUntil) : 0
  d.doneForTonight = obj.doneForTonight === true
  d.windDownDone = obj.windDownDone === true
  d.bedtimeDone = obj.bedtimeDone === true
  d.lateCount = clampInt(obj.lateCount, 0, 10) || 0
  d.urgentDone = obj.urgentDone === true
  d.morningDone = obj.morningDone === true
  return d
}

function withChanges(day, changes) {
  var out = {}
  for (var k in day) out[k] = day[k]
  for (var c in changes) out[c] = changes[c]
  return out
}

// 125 -> [2, 5] (hours, minutes) so the caller can localize.
function hoursMinutes(totalMinutes) {
  var t = Math.max(0, Math.round(Number(totalMinutes) || 0))
  return [Math.floor(t / 60), t % 60]
}

// ---------------------------------------------------------------------------
// Sleep schedule: bedtime comes from wake time, sleep need and time to fall asleep
// ---------------------------------------------------------------------------
var SLEEP_NEEDS = ["6h", "6h30", "7h", "7h30", "8h", "8h30", "9h", "9h30", "10h"]

// "7h30" -> 450; anything else -> 8h
function parseSleepNeed(label) {
  var m = /^(\d{1,2})h(\d{2})?$/.exec(String(label || "").trim())
  if (!m) return 480
  var v = parseInt(m[1], 10) * 60 + (m[2] ? parseInt(m[2], 10) : 0)
  return v >= 240 && v <= 720 ? v : 480
}

function normalizeWake(value) {
  var v = parseClock(value)
  return v === null ? 7 * 60 : v
}

function normalizeFallAsleep(v) {
  var n = clampInt(v, 0, 90)
  return n === null ? 15 : n
}

function bedtimeFor(wake, need, fallAsleep) {
  return ((wake - need - fallAsleep) % 1440 + 1440) % 1440
}

// Minutes of sleep you'd get if you went to bed now, before the next wake time.
function sleepIfBedNow(date, wake, fallAsleep) {
  var untilWake = wake - minutesOfDay(date)
  if (untilWake <= 0) untilWake += 1440
  return Math.max(0, Math.floor(untilWake - fallAsleep))
}

// ---------------------------------------------------------------------------
// Sleep history (from the strap's sleep sessions)
// ---------------------------------------------------------------------------
var DEBT_NIGHTS = 7
var MAX_PAYBACK = 120          // a long night repays at most 2h of debt

function parseNight(o) {
  if (!o || typeof o !== "object") return null
  var start = Number(o.start), end = Number(o.end)
  // Plausible epoch seconds (2020..2100) and a night under 20 hours.
  if (!(start > 1577836800 && start < 4102444800 && end > start && end - start < 20 * 3600)) return null
  var out = { start: start, end: end }
  var keys = ["asleep", "deep", "light", "rem", "awake"]
  for (var i = 0; i < keys.length; i++) {
    var v = clampInt(o[keys[i]], 0, 1200)
    if (v === null) return null
    out[keys[i]] = v
  }
  var score = clampInt(o.score, 0, 100)
  out.score = score === null ? 0 : score
  // Measured minutes from lying down to sleep onset; bounded = only a minimum.
  var lat = o.latency === undefined || o.latency === null ? null : clampInt(o.latency, 0, 120)
  if (lat !== null) { out.latency = lat; out.latencyBounded = o.latencyBounded === true }
  return out
}

// The most recent night that ended in the last 20 hours, or null.
function lastNight(nights, nowMs) {
  var best = null
  for (var i = 0; i < nights.length; i++) {
    var n = nights[i]
    if (n.end * 1000 <= nowMs && nowMs - n.end * 1000 < 20 * 3600000 && (!best || n.end > best.end)) best = n
  }
  return best
}

// ---------------------------------------------------------------------------
// Sleep coach. Rules follow the research summary in README.md:
// running-balance debt (lambda 0.93/night, 2h max payback per night, floor 0),
// debt levels, time-in-bed bedtime with gradual payback, regularity.
// ---------------------------------------------------------------------------
var DEBT_LAMBDA = 0.93
var KEEP_DAYS = 30
var WINDOW_DAYS = 14
var CALIBRATE_NIGHTS = 5
var DAY_MS = 86400000

function localDateKey(ms) {
  var d = new Date(ms)
  return d.getFullYear() + "-" + pad2(d.getMonth() + 1) + "-" + pad2(d.getDate())
}

function addDays(key, n) {
  var p = key.split("-")
  return localDateKey(new Date(+p[0], +p[1] - 1, +p[2] + n, 12).getTime())
}

// Keep a local history beyond the strap's ~8 nights: fresh nights replace
// stored ones they overlap; anything older than KEEP_DAYS is dropped.
function mergeNights(stored, fresh, nowMs) {
  // A re-fetched night keeps the latency measured for it earlier.
  fresh = fresh.map(function(f) {
    if (f.latency !== undefined) return f
    for (var i = 0; i < (stored || []).length; i++) {
      var o = stored[i]
      if (o.start === f.start && o.latency !== undefined)
        return withChanges(f, { latency: o.latency, latencyBounded: o.latencyBounded })
    }
    return f
  })
  var out = (stored || []).filter(function(n) {
    for (var i = 0; i < fresh.length; i++)
      if (!(n.end <= fresh[i].start || n.start >= fresh[i].end)) return false
    return true
  }).concat(fresh)
  out = out.filter(function(n) { return nowMs - n.end * 1000 < KEEP_DAYS * DAY_MS })
  out.sort(function(a, b) { return a.start - b.start })
  return out.slice(-60)
}

function parseNightList(text) {
  var arr
  try { arr = JSON.parse(String(text || "")) } catch (e) { return [] }
  if (!Array.isArray(arr)) return []
  var out = []
  for (var i = 0; i < arr.length && out.length < 60; i++) {
    var n = parseNight(arr[i])
    if (n) out.push(n)
  }
  return out
}

function recentNights(nights, nowMs, days) {
  return nights.filter(function(n) { return n.end * 1000 <= nowMs && nowMs - n.end * 1000 < days * DAY_MS })
}

// Running balance, oldest night first:
//   D = max(0, lambda * D + max(need - asleep, -MAX_PAYBACK))
// A day without a recorded night only decays. Nights are grouped by the local
// date they ended (naps ending the same day add up).
function debtBalance(nights, need, nowMs) {
  var byDay = {}
  var keys = []
  for (var i = 0; i < nights.length; i++) {
    var n = nights[i]
    if (n.end * 1000 > nowMs) continue
    var k = localDateKey(n.end * 1000)
    if (!(k in byDay)) { byDay[k] = 0; keys.push(k) }
    byDay[k] += n.asleep
  }
  var counted = recentNights(nights, nowMs, WINDOW_DAYS).length
  var result = { minutes: 0, level: "low", calibrating: counted < CALIBRATE_NIGHTS,
                 nights: counted, trend: "steady", series: [] }
  if (keys.length === 0) return result
  keys.sort()
  var today = localDateKey(nowMs)
  // Today only counts once its night is in, or it is afternoon and it never came.
  var last = (today in byDay || new Date(nowMs).getHours() >= 12) ? today : addDays(today, -1)
  var first = keys[0] < addDays(today, -KEEP_DAYS) ? addDays(today, -KEEP_DAYS) : keys[0]
  var d = 0
  for (var key = first; key <= last; key = addDays(key, 1)) {
    if (key in byDay) d = Math.max(0, DEBT_LAMBDA * d + Math.max(-MAX_PAYBACK, need - byDay[key]))
    else d = DEBT_LAMBDA * d
    result.series.push(Math.round(d))
  }
  result.minutes = Math.round(d)
  result.level = d < 120 ? "low" : d < 300 ? "moderate" : "high"
  if (result.series.length >= 4) {
    var diff = d - result.series[result.series.length - 4]
    result.trend = diff > 15 ? "rising" : diff < -15 ? "falling" : "steady"
  }
  return result
}

// Circular mean / standard deviation of clock times (minutes after midnight).
function circularStats(minutes) {
  if (!minutes || minutes.length < 2) return null
  var c = 0, s = 0
  for (var i = 0; i < minutes.length; i++) {
    var a = minutes[i] / 1440 * 2 * Math.PI
    c += Math.cos(a); s += Math.sin(a)
  }
  c /= minutes.length; s /= minutes.length
  var r = Math.min(1, Math.sqrt(c * c + s * s))
  var mean = Math.atan2(s, c) / (2 * Math.PI) * 1440
  return { mean: (mean + 1440) % 1440, sd: r > 0 ? Math.sqrt(-2 * Math.log(r)) * 1440 / (2 * Math.PI) : 720 }
}

function clockOf(epochSeconds) {
  var d = new Date(epochSeconds * 1000)
  return d.getHours() * 60 + d.getMinutes()
}

function circularDiff(a, b) {
  var d = Math.abs(a - b) % 1440
  return d > 720 ? 1440 - d : d
}

// Regularity over the last 14 days: spread of bedtimes and wake times, and
// social jetlag (mid-sleep on free mornings, Sat/Sun, vs. work mornings).
function regularity(nights, nowMs) {
  var recent = recentNights(nights, nowMs, WINDOW_DAYS)
  var out = { onsetSd: null, wakeSd: null, jetlag: null, irregular: false, jetlagFlag: false, nights: recent.length }
  if (recent.length < CALIBRATE_NIGHTS) return out
  out.onsetSd = Math.round(circularStats(recent.map(function(n) { return clockOf(n.start) })).sd)
  out.wakeSd = Math.round(circularStats(recent.map(function(n) { return clockOf(n.end) })).sd)
  var free = [], work = []
  recent.forEach(function(n) {
    var mid = clockOf((n.start + n.end) / 2)
    var wd = new Date(n.end * 1000).getDay()
    ;(wd === 0 || wd === 6 ? free : work).push(mid)
  })
  if (free.length >= 2 && work.length >= 2)
    out.jetlag = Math.round(circularDiff(circularStats(free).mean, circularStats(work).mean))
  out.irregular = out.onsetSd > 60
  out.jetlagFlag = out.jetlag !== null && out.jetlag >= 60
  return out
}

function median(values) {
  if (!values.length) return null
  var v = values.slice().sort(function(a, b) { return a - b })
  var m = Math.floor(v.length / 2)
  return v.length % 2 ? v[m] : (v[m - 1] + v[m]) / 2
}

function round15(m) { return Math.round(m / 15) * 15 }

// Tonight's plan. Time in bed = need + payback + time to fall asleep + the
// typical time awake during the night. Payback spreads debt over ~4 nights
// (max 60 min), and never puts bedtime more than an hour before the usual one.
function planNight(wake, need, fallAsleep, nights, debtMinutes, nowMs) {
  var recent = recentNights(nights, nowMs, WINDOW_DAYS)
  var waso = recent.length ? Math.max(0, Math.min(60, Math.round(median(recent.map(function(n) { return n.awake }))))) : 0
  var week = recentNights(nights, nowMs, 7)
  var habitual = week.length >= 3 ? Math.round(circularStats(week.map(function(n) { return clockOf(n.start) })).mean) : null
  var base = need + fallAsleep + waso
  var payback = debtMinutes < 120 ? 0 : Math.min(round15(debtMinutes / 4), 60)
  if (habitual !== null) {
    var habitualBeforeWake = ((wake - habitual) % 1440 + 1440) % 1440
    payback = Math.max(0, Math.min(payback, Math.floor((habitualBeforeWake + 60 - base) / 15) * 15))
  }
  return { bedtime: ((wake - base - payback) % 1440 + 1440) % 1440, payback: payback, waso: waso, habitual: habitual }
}

// Evening sequence: wind-down at -60 min, bedtime notice, reminders at +30
// and +90 min, then silence. Only while the user is active at the computer.
// state: { windDownDone, bedtimeDone, lateCount, lastAlertAt, snoozedUntil, doneForTonight }
function eveningAction(now, bedtime, active, state) {
  if (!active || state.doneForTonight) return "none"
  if (state.snoozedUntil && now.getTime() < state.snoozedUntil) return "none"
  var delta = minutesPastBedtime(now, bedtime)
  if (delta >= -60 && delta < 0) return state.windDownDone ? "none" : "wind-down"
  if (delta >= 0 && delta < 30) return state.bedtimeDone ? "none" : "bedtime"
  if (delta >= 30 && delta < LATE_WINDOW_MINUTES) {
    if (state.lastAlertAt && now.getTime() - state.lastAlertAt < 20 * 60000) return "none"
    var count = state.lateCount || 0
    if (count === 0 || (count === 1 && delta >= 90)) return "late"
  }
  return "none"
}

// One morning summary, on the first activity after a night that just ended.
function morningAction(now, active, state, night) {
  if (!active || state.morningDone || !night) return "none"
  var h = now.getHours()
  if (h < 5 || h >= 13) return "none"
  return now.getTime() - night.end * 1000 < 12 * 3600000 ? "morning" : "none"
}

// ---------------------------------------------------------------------------
// Weekly wake schedule: one setting per weekday, each "HH:MM", "free", or
// empty (the default time). A free morning aims for at most a 1h lie-in after
// the usual wake time, to limit social jetlag.
// ---------------------------------------------------------------------------
var WEEKDAYS = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"]
var WAKE_KEYS = ["wakeSun", "wakeMon", "wakeTue", "wakeWed", "wakeThu", "wakeFri", "wakeSat"]
var DEFAULT_WAKE = 7 * 60
var FREE_LIE_IN = 60

// values: 7 raw settings indexed Sun..Sat -> { days: [minutes | "free"], errors: [weekday index] }
function scheduleFromDays(values) {
  var days = [], errors = []
  for (var i = 0; i < 7; i++) {
    var raw = values && values[i] !== undefined && values[i] !== null ? String(values[i]).trim() : ""
    if (raw === "") { days.push(DEFAULT_WAKE); continue }
    if (/^(free|libre|off|-)$/i.test(raw)) { days.push("free"); continue }
    var t = parseClock(raw)
    if (t === null) { errors.push(i); days.push(DEFAULT_WAKE) } else days.push(t)
  }
  return { days: days, errors: errors }
}

// The morning a moment plans for: tomorrow from noon on, else today.
function targetMorning(date) {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate() + (date.getHours() >= 12 ? 1 : 0))
}

// -> { wake (minutes), free (bool), weekday (0-6) } for the coming morning.
function wakeTarget(date, schedule) {
  var morning = targetMorning(date)
  var weekday = morning.getDay()
  var entry = schedule.days[weekday]
  if (entry !== "free") return { wake: entry, free: false, weekday: weekday }
  var fixed = schedule.days.filter(function(v) { return v !== "free" })
  var usual = fixed.length ? median(fixed) : 7 * 60
  return { wake: (usual + FREE_LIE_IN) % 1440, free: true, weekday: weekday }
}

// Attach measured latencies to the stored nights (matched by sleep onset).
function applyLatency(nights, list) {
  return nights.map(function(n) {
    for (var i = 0; i < list.length; i++)
      if (list[i].start === n.start) return withChanges(n, { latency: list[i].latency, latencyBounded: list[i].bounded })
    return n
  })
}

// Typical time to fall asleep: median over the last 14 days of nights whose
// measurement is complete (not just a lower bound). null until CALIBRATE_NIGHTS.
function typicalLatency(nights, nowMs) {
  var recent = recentNights(nights, nowMs, WINDOW_DAYS)
  var full = recent.filter(function(n) { return n.latency !== undefined && !n.latencyBounded })
  var measured = recent.filter(function(n) { return n.latency !== undefined }).length
  return { minutes: full.length >= CALIBRATE_NIGHTS ? Math.round(median(full.map(function(n) { return n.latency }))) : null,
           nights: full.length, measured: measured }
}

// ---------------------------------------------------------------------------
// Movement: a "move" is moveSteps steps within MOVE_WINDOW minutes. It resets
// the sitting timer, and is the proof that clears a move reminder.
// ---------------------------------------------------------------------------
var MOVE_WINDOW_MS = 10 * 60000
var MOVE_MAX_NUDGES = 2
var MOVE_NUDGE_GAP_MS = 25 * 60000

// Keep (time, steps) readings from the last MOVE_WINDOW, plus the newest one
// before it as a baseline: when the strap was out of range (a walk away from
// the desk), the steps taken meanwhile show up as one jump on reconnect, and
// they still count. A lower count means the strap's counter reset (midnight),
// so start over.
function recordSteps(samples, steps, nowMs) {
  var kept = samples.filter(function(s) { return nowMs - s.t <= MOVE_WINDOW_MS })
  var older = samples.filter(function(s) { return nowMs - s.t > MOVE_WINDOW_MS })
  if (older.length) kept.unshift(older[older.length - 1])
  if (kept.length && steps < kept[kept.length - 1].steps) kept = []
  kept.push({ t: nowMs, steps: steps })
  return kept
}

function movedRecently(samples, moveSteps) {
  if (samples.length < 2) return false
  var lo = samples[0].steps
  for (var i = 1; i < samples.length; i++) lo = Math.min(lo, samples[i].steps)
  return samples[samples.length - 1].steps - lo >= moveSteps
}

// Machines without a kept hardware clock (Apple Silicon) boot at 1970 until NTP
// corrects it, so a timestamp taken at shell start can be decades old. Anything
// before 2020 is "clock not set yet": once the real time arrives, start over from it.
var CLOCK_SANE_MS = Date.UTC(2020, 0, 1)

function clockSet(ms) { return ms >= CLOCK_SANE_MS }

function rebaseUnsetClock(ms, nowMs) {
  return !clockSet(ms) && clockSet(nowMs) ? nowMs : ms
}

// s: { lastMovedAt, nudges, lastNudgeAt }  o: { everyMin, active, allowed }
function moveAction(nowMs, s, o) {
  if (!o.allowed || !o.active) return "none"
  if (nowMs - s.lastMovedAt < o.everyMin * 60000) return "none"
  if (s.nudges >= MOVE_MAX_NUDGES) return "none"
  if (s.lastNudgeAt && nowMs - s.lastNudgeAt < MOVE_NUDGE_GAP_MS) return "none"
  return "nudge"
}

function normalizeMoveEvery(v) { var n = clampInt(v, 20, 180); return n === null ? 50 : n }
function normalizeMoveSteps(v) { var n = clampInt(v, 10, 1000); return n === null ? 10 : n }

// Today's bars: the strap's per-hour counts, plus live steps not in them yet
// added to the current hour.
function hourBars(hourly, steps, nowMs) {
  var today = localDateKey(nowMs)
  var hours = hourly && hourly.day === today ? hourly.hours.slice() : [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
  var counted = hours.reduce(function(a, b) { return a + b }, 0)
  if (steps > counted) hours[new Date(nowMs).getHours()] += steps - counted
  return hours
}

// Which panel view fits the time: the sleep coach from 2h before bedtime
// until the late window ends, movement otherwise.
function autoView(pastBedtime) {
  return pastBedtime >= -120 && pastBedtime < LATE_WINDOW_MINUTES ? "sleep" : "move"
}
