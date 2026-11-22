// node tests/model.test.js
const fs = require("fs")
const path = require("path")
const assert = require("assert")

const src = fs.readFileSync(path.join(__dirname, "..", "Model.js"), "utf8").replace(".pragma library", "")
const M = new Function(src + "; return { parseLine, zoneFor, zoneBounds, normalizeMaxHr, emptyStats, addReading, average, pushPoint, rmssd, pushRR, formatDuration, batteryIcon, normalizeStepGoal, normalizeBarMode, goalProgress, parseClock, normalizeBedtime, formatClock, dayKey, minutesPastBedtime, freshDay, loadDay, withChanges, hoursMinutes, parseSleepNeed, normalizeWake, normalizeFallAsleep, bedtimeFor, sleepIfBedNow, SLEEP_NEEDS, parseNight, lastNight, mergeNights, parseNightList, debtBalance, circularStats, regularity, planNight, eveningAction, morningAction, localDateKey, scheduleFromDays, wakeTarget, targetMorning, WAKE_KEYS, applyLatency, typicalLatency, recordSteps, movedRecently, lastMoveInMinutes, moveAction, normalizeMoveEvery, normalizeMoveSteps, hourBars, autoView, clockSet, rebaseUnsetClock, minutesUntilBedtime, normalizeAddress, versionLess, MAX_POINTS, MAX_NAME };")()

// Heart rate lines
let ev = M.parseLine('{"t":"hr","bpm":72,"contact":null,"rr":[812,830]}')
assert.deepStrictEqual(ev, { type: "hr", bpm: 72, valid: true, contact: null, rr: [812, 830] })
assert.strictEqual(M.parseLine('{"t":"hr","bpm":0,"contact":null,"rr":[]}').valid, false)
assert.strictEqual(M.parseLine('{"t":"hr","bpm":80,"contact":false,"rr":[]}').valid, false)
assert.strictEqual(M.parseLine('{"t":"hr","bpm":500}'), null)
assert.strictEqual(M.parseLine('{"t":"hr","bpm":"x"}'), null)
assert.deepStrictEqual(M.parseLine('{"t":"hr","bpm":70,"rr":[5,900,99999,"a"]}').rr, [900])
assert.strictEqual(M.parseLine('{"t":"hr","bpm":70,"rr":' + JSON.stringify(Array(40).fill(800)) + '}').rr.length, 16)

// Other lines
assert.deepStrictEqual(M.parseLine('{"t":"battery","level":66}'), { type: "battery", level: 66 })
assert.strictEqual(M.parseLine('{"t":"battery","level":140}'), null)
assert.strictEqual(M.parseLine('{"t":"status","state":"rm -rf"}'), null)
assert.strictEqual(M.parseLine("not json"), null)
assert.strictEqual(M.parseLine(""), null)
assert.strictEqual(M.parseLine("x".repeat(5000)), null)
assert.strictEqual(M.parseLine("null"), null)
assert.strictEqual(M.parseLine('{"t":"unknown"}'), null)

// Markup-shaped and oversized device names stay verbatim text, bounded and
// free of control characters (the QML renders them as PlainText).
const evil = '<img src="https://example.invalid/t"><b>Strap</b>'
ev = M.parseLine(JSON.stringify({ t: "status", state: "connected", name: evil }))
assert.strictEqual(ev.name, evil)
ev = M.parseLine(JSON.stringify({ t: "status", state: "connected", name: "a\u0000b\nc" + "z".repeat(200) }))
assert.ok(ev.name.startsWith("abc"))
assert.ok(ev.name.length <= M.MAX_NAME)
assert.strictEqual(M.parseLine(JSON.stringify({ t: "info", firmware: "<i>3.27</i>" })).firmware, "<i>3.27</i>")

// Zones (max 190): 95 = 50%, 114 = 60%, 133 = 70%, 152 = 80%, 171 = 90%
assert.strictEqual(M.zoneFor(70, 190), 0)
assert.strictEqual(M.zoneFor(95, 190), 1)
assert.strictEqual(M.zoneFor(133, 190), 3)
assert.strictEqual(M.zoneFor(171, 190), 5)
assert.strictEqual(M.zoneFor(250, 190), 5)
assert.deepStrictEqual(M.zoneBounds(1, 190), [95, 113])
assert.deepStrictEqual(M.zoneBounds(5, 190), [171, 190])
assert.strictEqual(M.normalizeMaxHr("abc"), 190)
assert.strictEqual(M.normalizeMaxHr(50), 190)
assert.strictEqual(M.normalizeMaxHr(180), 180)

// Stats
let s = M.emptyStats()
s = M.addReading(s, 70, 0, 1000)
s = M.addReading(s, 80, 1, 2000)
s = M.addReading(s, 60, 0, 60000) // a long gap only credits 5 s
assert.deepStrictEqual([s.count, s.min, s.max, M.average(s)], [3, 60, 80, 70])
assert.strictEqual(s.zoneSeconds[1], 1)
assert.strictEqual(s.zoneSeconds[0], 5)

// Chart buffer
let pts = []
for (let i = 0; i < M.MAX_POINTS + 30; i++) pts = M.pushPoint(pts, i)
assert.strictEqual(pts.length, M.MAX_POINTS)
assert.strictEqual(pts[pts.length - 1], M.MAX_POINTS + 29)

// HRV
assert.strictEqual(M.rmssd([800, 810]), 0)
assert.strictEqual(M.rmssd([800, 810, 800, 810]), 10)
assert.strictEqual(M.pushRR(Array(59).fill(800), [1, 2, 3]).length, 60)

// Formatting
assert.strictEqual(M.formatDuration(272), "4:32")
assert.strictEqual(M.formatDuration(3729), "1:02:09")
assert.strictEqual(M.batteryIcon(100), "󰁹")
assert.strictEqual(M.batteryIcon(-1), "󰂑")

// Steps
assert.deepStrictEqual(M.parseLine('{"t":"steps","steps":4321,"distance":3100,"calories":150}'),
  { type: "steps", steps: 4321, distance: 3100, calories: 150 })
assert.deepStrictEqual(M.parseLine('{"t":"steps","steps":12}'), { type: "steps", steps: 12, distance: null, calories: null })
assert.strictEqual(M.parseLine('{"t":"steps","steps":-1}'), null)
assert.strictEqual(M.parseLine('{"t":"steps","steps":9999999}'), null)
assert.strictEqual(M.parseLine('{"t":"steps","steps":10,"distance":-5}').distance, null)
assert.deepStrictEqual(M.parseLine('{"t":"steps-status","state":"bad-key"}'), { type: "steps-status", state: "bad-key" })
assert.strictEqual(M.parseLine('{"t":"steps-status","state":"<b>x</b>"}'), null)
assert.strictEqual(M.normalizeStepGoal("x"), 8000)
assert.strictEqual(M.normalizeStepGoal(10000), 10000)
assert.strictEqual(M.normalizeBarMode("Steps"), "steps")
assert.strictEqual(M.normalizeBarMode("Heart rate"), "hr")
assert.strictEqual(M.normalizeBarMode("toString"), "both")
assert.strictEqual(M.goalProgress(4000, 8000), 0.5)
assert.strictEqual(M.goalProgress(20000, 8000), 1)
assert.strictEqual(M.goalProgress(-1, 8000), 0)

// Working late
assert.strictEqual(M.parseClock("23:00"), 1380)
assert.strictEqual(M.parseClock("0:30"), 30)
assert.strictEqual(M.parseClock("22h45"), 1365)
for (const bad of ["24:00", "23:60", "late", "", null, "<b>23:00</b>"]) assert.strictEqual(M.parseClock(bad), null)
assert.strictEqual(M.normalizeBedtime("nope"), 1380)
assert.strictEqual(M.formatClock(1380), "23:00")
assert.strictEqual(M.formatClock(1440 + 30), "00:30")

const at = (h, m, day = 20) => new Date(2026, 10, day, h, m)
assert.strictEqual(M.dayKey(at(23, 30)), "2026-11-20")
assert.strictEqual(M.dayKey(at(2, 0, 21)), "2026-11-20")     // 2am belongs to the evening before
assert.strictEqual(M.dayKey(at(6, 0, 21)), "2026-11-21")
assert.strictEqual(M.minutesPastBedtime(at(23, 30), 1380), 30)
assert.strictEqual(M.minutesPastBedtime(at(1, 0, 21), 1380), 120)
assert.strictEqual(M.minutesPastBedtime(at(22, 0), 1380), -60)
assert.strictEqual(M.minutesPastBedtime(at(0, 45, 21), 30), 15)     // bedtime after midnight
assert.strictEqual(M.minutesPastBedtime(at(23, 50), 30), -40)

const st = M.freshDay("2026-11-20")

// State file
const saved = JSON.stringify(M.withChanges(st, { activeMinutes: 125, headsUpDone: true, doneForTonight: true }))
assert.strictEqual(M.loadDay(saved, "2026-11-20").activeMinutes, 125)
assert.strictEqual(M.loadDay(saved, "2026-11-21").activeMinutes, 0)        // new day starts fresh
assert.deepStrictEqual(M.loadDay("garbage", "d"), M.freshDay("d"))
assert.strictEqual(M.loadDay('{"day":"d","activeMinutes":99999,"doneForTonight":"yes"}', "d").activeMinutes, 0)
assert.strictEqual(M.loadDay('{"day":"d","doneForTonight":"yes"}', "d").doneForTonight, false)
assert.deepStrictEqual(M.hoursMinutes(125), [2, 5])

// Sleep schedule
assert.strictEqual(M.parseSleepNeed("8h"), 480)
assert.strictEqual(M.parseSleepNeed("7h30"), 450)
assert.strictEqual(M.parseSleepNeed("lots"), 480)
assert.strictEqual(M.parseSleepNeed("20h"), 480)
for (const n of M.SLEEP_NEEDS) assert.notStrictEqual(M.parseSleepNeed(n), null)
assert.strictEqual(M.normalizeWake("6:30"), 390)
assert.strictEqual(M.normalizeWake("x"), 420)
assert.strictEqual(M.normalizeFallAsleep(200), 15)
assert.strictEqual(M.bedtimeFor(420, 480, 15), 1365)                 // 07:00, 8h, 15 min -> 22:45
assert.strictEqual(M.formatClock(M.bedtimeFor(420, 480, 15)), "22:45")
assert.strictEqual(M.bedtimeFor(600, 480, 0), 120)                   // 10:00 wake -> 02:00
assert.strictEqual(M.sleepIfBedNow(at(23, 35), 420, 15), 430)        // 7h10
assert.strictEqual(M.sleepIfBedNow(at(1, 0, 21), 420, 15), 345)
assert.strictEqual(M.sleepIfBedNow(at(6, 50, 21), 420, 15), 0)

// Sleep history
const fri = { start: 1795215180, end: 1795237200, asleep: 365, deep: 64, light: 212, rem: 89, awake: 2, score: 65 }
const thu = { start: 1795131120, end: 1795151340, asleep: 337, deep: 72, light: 173, rem: 92, awake: 0, score: 60 }
const tue = { start: 1794946260, end: 1794987180, asleep: 676, deep: 154, light: 352, rem: 170, awake: 6, score: 72 }
ev = M.parseLine(JSON.stringify({ t: "sleep", nights: [tue, thu, fri, { start: 1, end: 2 }, "x", { ...fri, asleep: 99999 }] }))
assert.strictEqual(ev.type, "sleep")
assert.strictEqual(ev.nights.length, 3)
assert.strictEqual(M.parseLine('{"t":"sleep","nights":"nope"}'), null)
assert.strictEqual(M.parseNight({ ...fri, end: fri.start + 21 * 3600 }), null)
assert.strictEqual(M.parseNight({ ...fri, score: "x" }).score, 0)
const nowMs = (fri.end + 16 * 3600) * 1000   // Friday evening
assert.strictEqual(M.lastNight(ev.nights, nowMs).asleep, 365)
assert.strictEqual(M.lastNight(ev.nights, (fri.end + 21 * 3600) * 1000), null)
// ---- Sleep coach ----
// Build nights in local time: night(dayOffset from 2026-11-20, bed "HH:MM" the evening before, minutes asleep)
const NOW = new Date(2026, 10, 20, 20, 0).getTime()          // Friday 20:00
function night(daysAgo, bedH, bedM, asleep, awake = 10) {
  const start = new Date(2026, 10, 20 - daysAgo - (bedH >= 12 ? 1 : 0), bedH, bedM).getTime() / 1000
  return { start, end: start + (asleep + awake) * 60, asleep, deep: 60, light: asleep - 120, rem: 60, awake, score: 70 }
}

// Running balance in real hours: 1h short every night for 14 nights -> ~ (1-0.93^14)/(1-0.93) h
let ns = []
for (let i = 13; i >= 0; i--) ns.push(night(i, 23, 0, 420))
let dbt = M.debtBalance(ns, 480, NOW)
const expect14 = Math.round(60 * (1 - Math.pow(0.93, 14)) / 0.07)
assert.strictEqual(dbt.minutes, expect14)                  // ~547 min, i.e. real hours, not 3.5h
assert.strictEqual(dbt.level, "high")
assert.strictEqual(dbt.calibrating, false)
assert.strictEqual(dbt.trend, "rising")
// a 12h night repays at most 2h
const after = M.debtBalance(ns.concat([]).slice(0, 13).concat([night(0, 21, 0, 720)]), 480, NOW).minutes
const before = M.debtBalance(ns.slice(0, 13), 480, new Date(2026, 10, 19, 20, 0).getTime()).minutes
assert.ok(Math.abs(after - Math.max(0, 0.93 * before - 120)) <= 1, [after, before])
// missing nights only decay, never count as zero sleep
const gap = M.debtBalance([night(5, 23, 0, 300)], 480, NOW)
assert.strictEqual(gap.minutes, Math.round(180 * Math.pow(0.93, 5)))
assert.strictEqual(gap.calibrating, true)
// surplus cannot be banked before a debt exists (floor at 0 every night)
assert.strictEqual(M.debtBalance([night(3, 21, 0, 600), night(1, 23, 0, 420)], 480, NOW).minutes, Math.round(60 * 0.93))
assert.strictEqual(M.debtBalance([], 480, NOW).minutes, 0)
// levels
assert.strictEqual(M.debtBalance([night(0, 23, 0, 360)], 480, NOW).level, "moderate")  // 120 min

// Merging local history
const old = night(25, 23, 0, 400), dupOld = night(2, 23, 0, 300), freshDup = { ...night(2, 23, 0, 450) }
let merged = M.mergeNights([night(40, 23, 0, 400), old, dupOld], [freshDup], NOW)
assert.strictEqual(merged.length, 2)                        // 40-day-old dropped, overlap replaced
assert.strictEqual(merged[1].asleep, 450)
assert.deepStrictEqual(M.parseNightList(JSON.stringify(merged)), merged)
assert.deepStrictEqual(M.parseNightList("junk"), [])

// Circular statistics cope with bedtimes around midnight
const cs = M.circularStats([23 * 60 + 30, 30])
assert.ok(Math.abs(cs.mean - 0) < 1 || Math.abs(cs.mean - 1440) < 1, cs.mean)
assert.ok(cs.sd > 25 && cs.sd < 35, cs.sd)

// Regularity: steady bedtimes are regular; alternating 22:00 / 01:30 is not
const steady = [0, 1, 2, 3, 4, 5, 6].map(i => night(i, 23, 0, 450))
assert.strictEqual(M.regularity(steady, NOW).irregular, false)
assert.ok(M.regularity(steady, NOW).onsetSd < 5)
const wobbly = [0, 1, 2, 3, 4, 5, 6].map(i => i % 2 ? night(i, 22, 0, 450) : night(i, 1, 30, 450))
assert.strictEqual(M.regularity(wobbly, NOW).irregular, true)
assert.strictEqual(M.regularity(steady.slice(0, 3), NOW).onsetSd, null)       // not enough nights
// social jetlag: weekend mid-sleep 2h later than weekdays
const jl = []
for (let i = 0; i < 14; i++) {
  const wd = new Date(2026, 10, 20 - i).getDay()
  jl.push(wd === 0 || wd === 6 ? night(i, 1, 0, 450) : night(i, 23, 0, 450))
}
assert.strictEqual(M.regularity(jl, NOW).jetlag, 120)
assert.strictEqual(M.regularity(jl, NOW).jetlagFlag, true)

// Plan: time in bed includes typical awake time; payback only with debt >= 2h,
// spread over ~4 nights, max 60 min, and never > 1h before the usual bedtime.
let plan = M.planNight(420, 480, 15, steady, 0, NOW)          // usual onset 23:00, awake 10 min
assert.deepStrictEqual([plan.bedtime, plan.payback, plan.waso, plan.habitual], [1355, 0, 10, 1380])  // 22:35
plan = M.planNight(420, 480, 15, steady, 600, NOW)            // wants 60, but earliest is 23:00 - 1h = 22:00
assert.strictEqual(plan.payback, 30)                          // 22:35 -> 22:05 (rounded down, never past the cap)
assert.strictEqual(M.formatClock(plan.bedtime), "22:05")
const earlyBird = [0, 1, 2, 3, 4].map(i => night(i, 21, 30, 450))
assert.strictEqual(M.planNight(420, 480, 15, earlyBird, 200, NOW).payback, 45)   // 200/4 = 50 -> 45, cap not binding
assert.strictEqual(M.planNight(420, 480, 15, earlyBird, 100, NOW).payback, 0)    // under 2h: no payback
const lateSleeper = [0, 1, 2, 3, 4].map(i => night(i, 0, 30, 450))
plan = M.planNight(420, 480, 15, lateSleeper, 600, NOW)        // usual 00:30: earliest 23:30, base bed 22:55
assert.strictEqual(plan.payback, 0)
assert.strictEqual(M.planNight(420, 480, 15, [], 300, NOW).payback, 60)   // no history: just the debt rule
assert.strictEqual(M.planNight(420, 480, 15, [], 300, NOW).waso, 0)

// Evening sequence
const ev0 = M.freshDay("2026-11-20")
const bed = 1365  // 22:45
assert.strictEqual(M.eveningAction(at(21, 30), bed, true, ev0), "none")
assert.strictEqual(M.eveningAction(at(21, 50), bed, true, ev0), "wind-down")
assert.strictEqual(M.eveningAction(at(21, 50), bed, false, ev0), "none")
assert.strictEqual(M.eveningAction(at(22, 50), bed, true, ev0), "bedtime")
const afterBed = M.withChanges(ev0, { windDownDone: true, bedtimeDone: true, lastAlertAt: at(22, 50).getTime() })
assert.strictEqual(M.eveningAction(at(23, 10), bed, true, afterBed), "none")     // 20 min since last notice
assert.strictEqual(M.eveningAction(at(23, 16), bed, true, afterBed), "late")     // +31
const once = M.withChanges(afterBed, { lateCount: 1, lastAlertAt: at(23, 16).getTime() })
assert.strictEqual(M.eveningAction(at(23, 50), bed, true, once), "none")         // waits for +90
assert.strictEqual(M.eveningAction(at(0, 16, 21), bed, true, once), "late")
const twice = M.withChanges(once, { lateCount: 2, lastAlertAt: at(0, 16, 21).getTime() })
assert.strictEqual(M.eveningAction(at(2, 0, 21), bed, true, twice), "none")      // then silence
assert.strictEqual(M.eveningAction(at(22, 50), bed, true, M.withChanges(ev0, { doneForTonight: true })), "none")
assert.strictEqual(M.eveningAction(at(22, 50), bed, true, M.withChanges(ev0, { snoozedUntil: at(23, 0).getTime() })), "none")
assert.strictEqual(M.loadDay(JSON.stringify(twice), "2026-11-20").lateCount, 2)

// Morning summary
const lastN = night(0, 23, 0, 450)    // ends ~06:40 on the 20th
const morningAt = new Date(2026, 10, 20, 8, 0)
assert.strictEqual(M.morningAction(morningAt, true, ev0, lastN), "morning")
assert.strictEqual(M.morningAction(morningAt, false, ev0, lastN), "none")
assert.strictEqual(M.morningAction(morningAt, true, M.withChanges(ev0, { morningDone: true }), lastN), "none")
assert.strictEqual(M.morningAction(new Date(2026, 10, 20, 15, 0), true, ev0, lastN), "none")
assert.strictEqual(M.morningAction(morningAt, true, ev0, null), "none")

// Weekly wake schedule (Sun..Sat)
let sch = M.scheduleFromDays(["free", "07:00", "07:00", "09:00", "07:00", "07:00", "free"])
assert.deepStrictEqual(sch.days, ["free", 420, 420, 540, 420, 420, "free"])
assert.deepStrictEqual(sch.errors, [])
assert.deepStrictEqual(M.scheduleFromDays([]).days, Array(7).fill(420))                 // unset: 07:00
assert.deepStrictEqual(M.scheduleFromDays(["", " 6:30 ", "FREE", "libre", null, undefined, "8h15"]).days,
  [420, 390, "free", "free", 420, 420, 495])
const bad = M.scheduleFromDays(["25:00", "<b>7</b>", "07:00", "", "", "", ""])
assert.deepStrictEqual(bad.errors, [0, 1])
assert.deepStrictEqual(bad.days.slice(0, 2), [420, 420])
assert.strictEqual(M.WAKE_KEYS[3], "wakeWed")
// Which morning: Friday 20 Nov 2026 evening plans for Saturday; Tuesday evening for Wednesday
assert.strictEqual(M.targetMorning(new Date(2026, 10, 20, 22, 0)).getDay(), 6)
assert.strictEqual(M.targetMorning(new Date(2026, 10, 21, 1, 0)).getDay(), 6)          // after midnight: same morning
assert.deepStrictEqual(M.wakeTarget(new Date(2026, 10, 17, 21, 0), sch), { wake: 540, free: false, weekday: 3 })   // Tue -> Wed 09:00
assert.deepStrictEqual(M.wakeTarget(new Date(2026, 10, 19, 21, 0), sch), { wake: 420, free: false, weekday: 5 })   // Thu -> Fri
assert.deepStrictEqual(M.wakeTarget(new Date(2026, 10, 20, 21, 0), sch), { wake: 480, free: true, weekday: 6 })    // Fri -> Sat free: 07:00 + 1h
assert.deepStrictEqual(M.wakeTarget(new Date(2026, 10, 21, 9, 0), sch).weekday, 6)

// Time to fall asleep
ev = M.parseLine(JSON.stringify({ t: "latency", nights: [{ start: fri.start, latency: 9, bounded: true }, { start: 5, latency: 3 }, { start: thu.start, latency: 500 }, "x"] }))
assert.deepStrictEqual(ev, { type: "latency", nights: [{ start: fri.start, latency: 9, bounded: true }] })
let lat = M.applyLatency([fri, thu], ev.nights)
assert.strictEqual(lat[0].latency, 9); assert.strictEqual(lat[0].latencyBounded, true); assert.strictEqual(lat[1].latency, undefined)
assert.strictEqual(M.parseNight(lat[0]).latency, 9)                                   // survives the state file
assert.strictEqual(M.parseNight({ ...fri, latency: 999 }).latency, undefined)
// a re-fetched night (no latency) keeps the stored measurement
const refetched = M.mergeNights(lat, [{ ...fri }], (fri.end + 3600) * 1000)
assert.strictEqual(refetched.find(n => n.start === fri.start).latency, 9)
// median of complete measurements only, after 5 nights
const latNights = [3, 2, 1, 0].map((d, i) => ({ ...night(d, 23, 0, 450), latency: [10, 20, 14, 30][i] }))
assert.deepStrictEqual(M.typicalLatency(latNights, NOW), { minutes: null, nights: 4, measured: 4 })
const five = latNights.concat([{ ...night(4, 23, 0, 450), latency: 12 }, { ...night(5, 23, 0, 450), latency: 2, latencyBounded: true }])
assert.deepStrictEqual(M.typicalLatency(five, NOW), { minutes: 14, nights: 5, measured: 6 })

// Movement
let smp = []
const T = new Date(2026, 10, 20, 14, 0).getTime()
smp = M.recordSteps(smp, 1000, T)
smp = M.recordSteps(smp, 1040, T + 60000)
assert.strictEqual(M.movedRecently(smp, 100), false)
smp = M.recordSteps(smp, 1110, T + 4 * 60000)
assert.strictEqual(M.movedRecently(smp, 100), true)                       // 110 steps in 4 min
smp = M.recordSteps(smp, 1115, T + 20 * 60000)                             // old readings drop out, newest stays as baseline
assert.strictEqual(smp.length, 2); assert.strictEqual(M.movedRecently(smp, 10), false)
assert.strictEqual(M.recordSteps(smp, 12, T + 21 * 60000).length, 1)      // counter reset at midnight
// Out of range all afternoon: the steps arrive as one jump on reconnect and count.
smp = M.recordSteps([], 3000, T)
smp = M.recordSteps(smp, 6500, T + 4 * 3600000)
assert.strictEqual(smp.length, 2); assert.strictEqual(M.movedRecently(smp, 10), true)
smp = M.recordSteps(smp, 6502, T + 5 * 3600000)                            // an hour later, sitting again
assert.strictEqual(M.movedRecently(smp, 10), false)
const mv = { lastMovedAt: T, nudges: 0, lastNudgeAt: 0 }
const on = { everyMin: 50, active: true, allowed: true }
assert.strictEqual(M.moveAction(T + 49 * 60000, mv, on), "none")
assert.strictEqual(M.moveAction(T + 50 * 60000, mv, on), "nudge")
assert.strictEqual(M.moveAction(T + 50 * 60000, mv, { ...on, active: false }), "none")
assert.strictEqual(M.moveAction(T + 50 * 60000, mv, { ...on, allowed: false }), "none")
const nudged = { ...mv, nudges: 1, lastNudgeAt: T + 50 * 60000 }
assert.strictEqual(M.moveAction(T + 70 * 60000, nudged, on), "none")       // 25 min gap
assert.strictEqual(M.moveAction(T + 76 * 60000, nudged, on), "nudge")
assert.strictEqual(M.moveAction(T + 200 * 60000, { ...nudged, nudges: 2 }, on), "none")   // then silence
assert.strictEqual(M.normalizeMoveEvery(5), 50); assert.strictEqual(M.normalizeMoveSteps(200), 200)
assert.strictEqual(M.normalizeMoveSteps(undefined), 10); assert.strictEqual(M.normalizeMoveSteps(3), 10)
smp = M.recordSteps([], 2000, T); smp = M.recordSteps(smp, 2012, T + 60000)
assert.strictEqual(M.movedRecently(smp, M.normalizeMoveSteps(undefined)), true)          // a dozen steps ends sitting
// Hourly bars
const todayKey = M.localDateKey(T)
const hrs = Array(24).fill(0); hrs[9] = 2000; hrs[13] = 500
let bars = M.hourBars({ day: todayKey, hours: hrs }, 2800, T)
assert.strictEqual(bars[14], 300); assert.strictEqual(bars[9], 2000)        // live extra goes to the current hour
assert.deepStrictEqual(M.hourBars({ day: "2000-01-01", hours: hrs }, 0, T), Array(24).fill(0))
assert.strictEqual(M.hourBars(null, 50, T)[14], 50)
const hev = M.parseLine(JSON.stringify({ t: "hourly", day: todayKey, hours: hrs }))
assert.strictEqual(hev.hours[9], 2000)
assert.strictEqual(M.parseLine(JSON.stringify({ t: "hourly", day: "nope", hours: hrs })), null)
assert.strictEqual(M.parseLine(JSON.stringify({ t: "hourly", day: todayKey, hours: [1] })), null)
// Per-minute log from the strap
const mev = M.parseLine(JSON.stringify({ t: "minutes", from: 1800000000, steps: [0, 3, "x", 999, -1] }))
assert.deepStrictEqual(mev, { type: "minutes", from: 1800000000000, steps: [0, 3, 0, 0, 0] })
assert.strictEqual(M.parseLine(JSON.stringify({ t: "minutes", from: 5, steps: [] })), null)
assert.strictEqual(M.parseLine(JSON.stringify({ t: "minutes", from: 1800000000, steps: "x" })), null)
const walk = Array(240).fill(0); for (let i = 60; i < 180; i++) walk[i] = 80   // walked from minute 60 to 180
assert.strictEqual(M.lastMoveInMinutes(T, walk, 10), T + 180 * 60000)
assert.strictEqual(M.lastMoveInMinutes(T, Array(240).fill(0), 10), 0)
const trickle = Array(60).fill(0); for (let i = 0; i < 60; i += 15) trickle[i] = 4  // 4 steps every 15 min: not a move
assert.strictEqual(M.lastMoveInMinutes(T, trickle, 10), 0)
// Auto view
assert.strictEqual(M.autoView(-180), "move")
assert.strictEqual(M.autoView(-120), "sleep")
assert.strictEqual(M.autoView(30), "sleep")
assert.strictEqual(M.autoView(400), "move")

console.log("model tests passed")

// Clock set after shell start (RTC at 1970 until NTP): stale start times rebase.
const real = Date.UTC(2026, 10, 21, 8, 0)
assert.strictEqual(M.clockSet(35000), false)
assert.strictEqual(M.clockSet(real), true)
assert.strictEqual(M.rebaseUnsetClock(35000, real), real)
assert.strictEqual(M.rebaseUnsetClock(35000, 60000), 35000)
assert.strictEqual(M.rebaseUnsetClock(real - 3600000, real), real - 3600000)

// Bedtime countdown: 09:00 with a 22:33 bedtime is 13h33 away, not "0 min".
const bedAt = 22 * 60 + 33
assert.strictEqual(M.minutesUntilBedtime(M.minutesPastBedtime(new Date(2026, 10, 21, 9, 0), bedAt)), 813)
assert.strictEqual(M.minutesUntilBedtime(M.minutesPastBedtime(new Date(2026, 10, 21, 21, 33), bedAt)), 60)

// Strap confirmation: only well-formed addresses, bounded candidate lists, plain names.
assert.strictEqual(M.normalizeAddress(" c3:c4:bd:b7:27:4c "), "C3:C4:BD:B7:27:4C")
for (const bad of ["", null, undefined, "C3:C4:BD:B7:27", "C3-C4-BD-B7-27-4C", "C3:C4:BD:B7:27:4G", "x".repeat(40)])
  assert.strictEqual(M.normalizeAddress(bad), "")
const cand = M.parseLine(JSON.stringify({ t: "candidates", devices: [
  { name: "<b>Helio</b>\u0007", address: "c3:c4:bd:b7:27:4c" }, { name: "x", address: "nope" }, "junk",
  ...Array.from({ length: 20 }, (_, i) => ({ name: "s" + i, address: "00:00:00:00:00:" + String(i).padStart(2, "0") }))] }))
assert.strictEqual(cand.type, "candidates")
assert.strictEqual(cand.devices.length, 8)
assert.deepStrictEqual(cand.devices[0], { name: "<b>Helio</b>", address: "C3:C4:BD:B7:27:4C" })
assert.strictEqual(M.parseLine('{"t":"candidates","devices":"x"}'), null)
assert.strictEqual(M.parseLine('{"t":"status","state":"unconfirmed"}').state, "unconfirmed")

assert.strictEqual(M.versionLess("1.0.0", "1.1.0"), true)
assert.strictEqual(M.versionLess("1.1.0", "1.1.0"), false)
assert.strictEqual(M.versionLess("1.10.0", "1.9.9"), false)
assert.strictEqual(M.versionLess("", "1.1.0"), true)
assert.strictEqual(M.versionLess("garbage", "1.1.0"), true)
