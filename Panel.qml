pragma ComponentBehavior: Bound

import QtQuick
import Quickshell
import Quickshell.Io
import Quickshell.Wayland
import qs.Commons
import qs.Commons as Commons
import qs.Ui
import "Model.js" as Model
import "intro"

// Helio Strap: live heart rate from an Amazfit Helio Strap (or any paired
// Bluetooth LE heart rate strap) in the bar. the helio-strap helper follows the strap over
// BlueZ and streams JSON lines; this widget keeps the session's statistics,
// heart rate zones and a ten-minute chart.
//
//   omarchy-shell lorismaz.helio-strap toggle
//   omarchy-shell lorismaz.helio-strap resetSession
//   omarchy-shell lorismaz.helio-strap intro          (play or stop the intro)
Panel {
  id: root
  moduleName: "lorismaz.helio-strap"
  ipcTarget: "lorismaz.helio-strap"
  manageIpc: false

  readonly property string deviceFilter: String(setting("deviceName", "Helio") || "")
  // The strap the user confirmed as theirs. Until then nothing is read from any
  // device: on a shared computer a paired strap may be someone else's.
  readonly property string confirmedAddress: Model.normalizeAddress(setting("deviceAddress", ""))
  property var candidates: []
  readonly property bool autoReconnect: setting("autoReconnect", true) !== false
  readonly property int maxHr: Model.normalizeMaxHr(setting("maxHeartRate", 190))
  readonly property bool showZoneInBar: setting("showZoneInBar", false) === true
  readonly property string barMode: Model.normalizeBarMode(setting("barShows", "Heart rate and steps"))
  readonly property int stepGoal: Model.normalizeStepGoal(setting("stepGoal", 8000))

  readonly property color foreground: bar ? bar.foreground : Commons.Color.foreground
  readonly property color urgent: bar ? bar.urgent : Commons.Color.urgent
  readonly property color accent: Commons.Color.accent
  readonly property color dim: Qt.darker(foreground, 1.4)
  readonly property string fontFamily: bar ? bar.fontFamily : Style.font.family

  // From helio-strap stream
  property string linkState: "searching"
  property string deviceName: ""
  property string firmware: ""
  property int battery: -1
  property int bpm: 0
  property double lastReadingAt: 0
  property var rrBuffer: []
  // Steps over the authenticated Zepp channel; -1 until the first reading.
  property int steps: -1
  property int distance: -1
  property int calories: -1
  property string stepsState: ""
  // Local sleep history (the strap only keeps ~8 nights), merged with each fetch.
  property var nights: []
  readonly property var lastNight: Model.lastNight(nights, now)
  readonly property var debt: Model.debtBalance(nights, sleepNeed, now)
  readonly property var plan: Model.planNight(wakeTime, sleepNeed, fallAsleep, nights, debt.calibrating ? 0 : debt.minutes, now)
  readonly property var regular: Model.regularity(nights, now)

  // Session
  property var stats: Model.emptyStats()
  property var points: []
  property double sessionStartedAt: Date.now()
  property double now: Date.now()
  property string helperError: ""

  readonly property bool live: linkState === "connected" && lastReadingAt > 0 && now - lastReadingAt < Model.STALE_MS
  readonly property int zone: live ? Model.zoneFor(bpm, maxHr) : 0
  readonly property int hrv: Model.rmssd(rrBuffer)
  readonly property string bpmText: live ? Commons.I18n.formatNumber(bpm, 0) : "--"

  Commons.Translations { id: strings; directory: Qt.resolvedUrl("i18n") }

  function zoneName(z) { return strings.t("zone." + z) }

  function zoneColor(z) {
    // Calm zones take the theme foreground, the hard ones the accent, the
    // top zone the urgent color.
    return z >= 5 ? root.urgent : z >= 3 ? root.accent : root.foreground
  }

  function handle(line) {
    var ev = Model.parseLine(line)
    if (!ev) return
    if (ev.type === "candidates") {
      root.candidates = ev.devices
    } else if (ev.type === "status") {
      root.linkState = ev.state
      if (ev.name !== "") root.deviceName = ev.name
      if (ev.state !== "connected") root.rrBuffer = []
    } else if (ev.type === "battery") {
      root.battery = ev.level
    } else if (ev.type === "info") {
      root.firmware = ev.firmware
    } else if (ev.type === "steps") {
      root.steps = ev.steps
      root.distance = ev.distance === null ? -1 : ev.distance
      root.calories = ev.calories === null ? -1 : ev.calories
      var t = Date.now()
      root.lastStepsAt = t
      root.stepSamples = Model.recordSteps(root.stepSamples, ev.steps, t)
      if (Model.movedRecently(root.stepSamples, root.moveSteps)) {
        // Proof of movement: clears a pending move reminder.
        if (root.moveNudges > 0) root.notify(strings.t("notify.moved.title"), strings.t("notify.moved.body"), false)
        root.lastMovedAt = t
        root.movedSeen = true
        root.moveNudges = 0
        root.lastNudgeAt = 0
      }
    } else if (ev.type === "minutes") {
      // Walks the live readings missed (strap out of range, or no key yet).
      var moved = Math.min(Model.lastMoveInMinutes(ev.from, ev.steps, root.moveSteps), Date.now())
      if (moved > root.lastMovedAt) {
        root.lastMovedAt = moved
        root.movedSeen = true
        root.moveNudges = 0
        root.lastNudgeAt = 0
      }
    } else if (ev.type === "hourly") {
      root.hourly = { day: ev.day, hours: ev.hours }
    } else if (ev.type === "latency") {
      root.nights = Model.applyLatency(root.nights, ev.nights)
      root.saveNights()
    } else if (ev.type === "sleep") {
      root.nights = Model.mergeNights(root.nights, ev.nights, Date.now())
      root.saveNights()
    } else if (ev.type === "steps-status") {
      root.stepsState = ev.state
    } else if (ev.type === "hr" && ev.valid) {
      var t = Date.now()
      root.bpm = ev.bpm
      root.lastReadingAt = t
      root.setNow(t)
      root.stats = Model.addReading(root.stats, ev.bpm, Model.zoneFor(ev.bpm, root.maxHr), t)
      if (ev.rr.length > 0) root.rrBuffer = Model.pushRR(root.rrBuffer, ev.rr)
    }
  }

  function resetSession() {
    stats = Model.emptyStats()
    points = []
    rrBuffer = []
    sessionStartedAt = Date.now()
  }

  readonly property bool needsSetup: stepsState === "no-key" || stepsState === "bad-key"

  // ---------- In-panel settings ----------
  property bool showSettings: false
  property string settingsTab: "sleep"
  readonly property string pluginVersion: "1.2.0"   // keep in step with manifest.json

  // ---------- Intro (Settings > About) ----------
  property bool introOn: false
  function playIntro() { root.close(); root.introOn = true }

  Loader {
    active: root.introOn
    sourceComponent: PanelWindow {
      anchors { top: true; bottom: true; left: true; right: true }
      exclusionMode: ExclusionMode.Ignore
      WlrLayershell.layer: WlrLayer.Overlay
      WlrLayershell.keyboardFocus: WlrKeyboardFocus.Exclusive
      WlrLayershell.namespace: "helio-strap-intro"
      color: Commons.Color.background
      Cracktro {
        anchors.fill: parent
        version: root.pluginVersion
        background: Commons.Color.background
        foreground: Commons.Color.foreground
        accent: Commons.Color.accent
        muted: Commons.Color.muted
        urgent: Commons.Color.urgent
        fontFamily: root.fontFamily
        onFinished: root.introOn = false
      }
    }
  }
  onOpenedChanged: if (!opened) { showSettings = false; viewChoice = "" }

  // Same path as `omarchy bar set`: persist into shell.json through the bar.
  function setSetting(key, value) {
    var next = Object.assign({}, root.settings)
    next[key] = value
    root.settings = next
    if (root.bar && root.bar.shell) root.bar.shell.updateEntryInline(root.moduleName, root.settings)
  }

  function wakeValue(key) {
    return String(setting(key, (key === "wakeSat" || key === "wakeSun") ? "free" : "07:00") || "")
  }

  function setupSteps() {
    root.close()
    Quickshell.execDetached(["omarchy-launch-tui", "--app-id=org.omarchy.helio-strap-setup",
                             root.helperPath, "setup", root.deviceFilter])
  }

  function openBluetooth() {
    root.close()
    Quickshell.execDetached(["omarchy-shell", "omarchy.bluetooth", "open"])
  }

  ShellIpc {
    target: "lorismaz.helio-strap"

    function open() { root.open() }
    function close() { root.close() }
    function show() { root.open() }
    function hide() { root.close() }
    function toggle() { root.toggle() }
    function resetSession() { root.resetSession() }
    function setupSteps() { root.setupSteps() }
    function installHelper() { root.installHelper() }
    function snooze() { root.snooze() }
    function doneForTonight() { root.toggleDoneForTonight() }
    function testEvening(kind: string) { root.sendEvening(kind, new Date()) }
    function testMorning() { root.sendMorning() }
    function openSettings() { root.open(); root.showSettings = true }
    function showView(name: string) { root.open(); root.showSettings = false; root.viewChoice = name === "sleep" ? "sleep" : "move" }
    function intro() { if (root.introOn) root.introOn = false; else root.playIntro() }
  }

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  // ---------- Helper process ----------
  // helio-strap is found on PATH (a package or a local build) or in the
  // plugin's own data folder, where install-helper.sh puts the release binary.
  property bool helperMissing: false
  property bool helperOutdated: false
  property string helperPath: ""
  property string helperVersion: ""
  // Oldest helper that follows only the confirmed strap and takes notifications on stdin.
  readonly property string helperRequired: "1.2.0"
  readonly property string installScript: Qt.resolvedUrl("install-helper.sh").toString().replace(/^file:\/\//, "")
  readonly property var helperCommand: [root.helperPath, "stream", root.deviceFilter, root.autoReconnect ? "1" : "0", root.confirmedAddress]
  onHelperCommandChanged: if (helperProc.running) helperProc.running = false   // restartTimer relaunches it

  function installHelper() {
    root.close()
    // The launcher runs its argument through bash: quote the script path.
    var quoted = "'" + root.installScript.replace(/'/g, "'\\''") + "'"
    Quickshell.execDetached(["omarchy-launch-floating-terminal-with-presentation", "bash " + quoted])
  }

  Process {
    id: helperCheck
    command: ["sh", "-c", "p=$(command -v helio-strap) || { p=\"${XDG_DATA_HOME:-$HOME/.local/share}/helio-strap/bin/helio-strap\"; [ -x \"$p\" ] || exit 1; }; echo \"$p\"; \"$p\" --version"]
    running: true
    stdout: SplitParser {
      onRead: function(line) {
        var l = String(line).trim()
        if (root.helperPath === "") root.helperPath = l
        else if (root.helperVersion === "") root.helperVersion = l.replace(/^helio-strap\s+/, "")
      }
    }
    onStarted: { root.helperPath = ""; root.helperVersion = "" }
    onExited: function(exitCode) {
      root.helperMissing = root.helperPath === ""
      root.helperOutdated = !root.helperMissing && Model.versionLess(root.helperVersion, root.helperRequired)
      if (root.helperMissing || root.helperOutdated) restartTimer.restart()
      else helperProc.running = true
    }
  }

  Process {
    id: helperProc
    command: root.helperCommand
    running: false
    stdinEnabled: true
    stdout: SplitParser { onRead: function(line) { root.handle(line) } }
    stderr: SplitParser {
      onRead: function(line) { root.helperError = String(line).slice(0, 200) }
    }
    onStarted: root.helperError = ""
    onExited: function(exitCode) {
      root.linkState = "searching"
      restartTimer.restart()
    }
  }

  Timer {
    id: restartTimer
    interval: 5000
    onTriggered: helperCheck.running = true
  }

  // One chart point every few seconds; also ages the "live" flag.
  Timer {
    interval: Model.SAMPLE_SECONDS * 1000
    running: true
    repeat: true
    onTriggered: {
      root.setNow(Date.now())
      if (root.live) root.points = Model.pushPoint(root.points, root.bpm)
    }
  }

  readonly property string stepsText: steps >= 0 ? Commons.I18n.formatNumber(steps, 0) : ""
  readonly property string barHrText: live
    ? (showZoneInBar && zone > 0 ? bpmText + " · Z" + zone : bpmText) : ""
  readonly property string barStepsText: stepsText !== "" ? "󰖃 " + stepsText : ""
  // Steps mode puts the walker in the icon slot, so the label is the bare number.
  readonly property string barLabel: barMode === "hr" ? barHrText
    : barMode === "steps" ? stepsText
    : [barStepsText, barHrText].filter(function(t) { return t !== "" }).join("   ")

  TextMetrics {
    id: barLabelMetrics
    font.family: root.fontFamily
    font.pixelSize: Style.bar.iconFont
    text: root.barLabel
  }

  // ---------- Sleep schedule and working late ----------
  readonly property var wakeSchedule: Model.scheduleFromDays(Model.WAKE_KEYS.map(function(k) {
    return setting(k, (k === "wakeSat" || k === "wakeSun") ? "free" : "07:00")
  }))
  // The coming morning's wake time: tonight's plan follows the weekly schedule.
  readonly property var target: Model.wakeTarget(nowDate, wakeSchedule)
  readonly property int wakeTime: target.wake
  readonly property int sleepNeed: Model.parseSleepNeed(setting("sleepNeed", "8h"))
  // Time to fall asleep: measured from the strap's minute data (Auto), or the manual value.
  readonly property int fallAsleepManual: Model.normalizeFallAsleep(setting("fallAsleep", 15))
  readonly property bool fallAsleepAuto: setting("fallAsleepAuto", true) !== false
  readonly property var latencyInfo: Model.typicalLatency(nights, now)
  readonly property bool fallAsleepMeasured: fallAsleepAuto && latencyInfo.minutes !== null
  readonly property int fallAsleep: fallAsleepMeasured ? latencyInfo.minutes : fallAsleepManual
  readonly property int bedtime: plan.bedtime
  readonly property bool lateAlerts: setting("lateAlerts", true) !== false

  readonly property string stateDir: Quickshell.env("HOME") + "/.local/state/helio-strap"
  property var day: Model.freshDay(Model.dayKey(new Date()))
  property bool dayLoaded: false

  readonly property var nowDate: new Date(root.now)
  readonly property int pastBedtime: Math.round(Model.minutesPastBedtime(nowDate, bedtime))
  readonly property int sleepIfNow: Model.sleepIfBedNow(nowDate, wakeTime, fallAsleep + plan.waso)
  readonly property bool lateNow: pastBedtime >= 0 && pastBedtime < 360
  // "" | "wind-down" (the hour before bedtime) | "late" (past bedtime)
  readonly property string moonPhase: !lateAlerts ? ""
    : lateNow ? "late"
    : (pastBedtime >= -60 && pastBedtime < 0) ? "wind-down" : ""

  // Also repairs start times taken before NTP set the clock (see Model.rebaseUnsetClock).
  function setNow(t) {
    root.lastMovedAt = Model.rebaseUnsetClock(root.lastMovedAt, t)
    root.sessionStartedAt = Model.rebaseUnsetClock(root.sessionStartedAt, t)
    root.now = t
  }

  function duration(minutes) {
    var hm = Model.hoursMinutes(minutes)
    if (hm[0] === 0) return strings.t("duration.m").arg(Commons.I18n.formatNumber(hm[1], 0))
    if (hm[1] === 0) return strings.t("duration.h").arg(Commons.I18n.formatNumber(hm[0], 0))
    return strings.t("duration.hm").arg(Commons.I18n.formatNumber(hm[0], 0)).arg(Model.pad2(hm[1]))
  }

  function debtText() {
    if (root.debt.calibrating) return strings.t("debt.calibrating").arg(root.debt.nights).arg(5)
    if (root.debt.minutes < 15) return strings.t("debt.none")
    return strings.t("debt.level." + root.debt.level).arg(root.duration(root.debt.minutes))
           + (root.debt.trend !== "steady" ? " · " + strings.t("debt.trend." + root.debt.trend) : "")
  }

  function sendEvening(action, now) {
    var wake = Model.formatClock(root.wakeTime)
    var bed = Model.formatClock(root.bedtime)
    var payback = root.plan.payback > 0 ? " " + strings.t("notify.payback").arg(root.duration(root.plan.payback)) : ""
    if (action === "wind-down") {
      root.notify(strings.t("notify.windDown.title").arg(bed), strings.t("notify.windDown.body") + payback, false)
      return {}
    }
    var left = Model.sleepIfBedNow(now, root.wakeTime, root.fallAsleep + root.plan.waso)
    var short = Math.max(0, root.sleepNeed - left)
    if (action === "bedtime") {
      root.notify(strings.t("notify.bedtime.title"), strings.t("notify.bedtime.body").arg(root.duration(left)).arg(wake), false)
      return {}
    }
    // Late: lead with what stopping now still gives; mention the shortfall once;
    // urgent only once a night, when an hour or more is already lost.
    var urgent = short >= 60 && !root.day.urgentDone && !root.target.free
    var body = strings.t("notify.late.body").arg(root.duration(left)).arg(wake)
    if (short > 0) body += " " + strings.t("notify.late.short").arg(root.duration(short)).arg(root.duration(root.sleepNeed))
    root.notify(strings.t("notify.late.title"), body, urgent)
    return urgent ? { urgentDone: true } : {}
  }

  function sendMorning() {
    var n = root.lastNight
    if (!n) return
    var diff = n.asleep - root.sleepNeed
    var lines = [diff >= 0 ? strings.t("morning.enough").arg(root.duration(root.sleepNeed))
                           : strings.t("morning.short").arg(root.duration(-diff)).arg(root.duration(root.sleepNeed)),
                 root.debtText()]
    if (root.regular.irregular) lines.push(strings.t("regular.irregular").arg(root.duration(root.regular.onsetSd)))
    else if (root.regular.jetlagFlag) lines.push(strings.t("regular.jetlag").arg(root.duration(root.regular.jetlag)))
    lines.push(strings.t("morning.tonight").arg(Model.formatClock(root.bedtime)))
    root.notify(strings.t("morning.title").arg(root.duration(n.asleep)), lines.join("\n"), false)
  }

  function clock(epochSeconds) {
    return new Date(epochSeconds * 1000).toLocaleTimeString(Qt.locale(Commons.I18n.formatLocale), "HH:mm")
  }

  // Sent through the helper over D-Bus: health details never go on a command
  // line, where other local accounts could read them.
  function notify(title, body, urgent) {
    if (helperProc.running)
      helperProc.write(JSON.stringify({ t: "notify", title: String(title), body: String(body), urgent: urgent === true }) + "\n")
  }

  function saveNights() {
    if (root.nightsPath !== "") nightsFile.setText(JSON.stringify(root.nights) + "\n")
  }

  function confirmStrap(address) {
    var a = Model.normalizeAddress(address)
    if (a !== "") root.setSetting("deviceAddress", a)
  }

  function forgetStrap() {
    root.setSetting("deviceAddress", "")
  }

  function saveDay(next) {
    root.day = next
    dayFile.setText(JSON.stringify(next) + "\n")
  }

  function tick() {
    var now = new Date()
    var key = Model.dayKey(now)
    var d = root.day.day === key ? root.day : Model.freshDay(key)
    var active = !activityMonitor.isIdle
    var changes = {}
    if (active) {
      changes.activeMinutes = Math.min(1440, d.activeMinutes + 1)
      if (!d.firstActiveAt) changes.firstActiveAt = now.getTime()
    }
    if (Model.morningAction(now, active, d, root.lastNight) === "morning") {
      root.sendMorning()
      changes.morningDone = true
    }
    var action = root.lateAlerts ? Model.eveningAction(now, root.bedtime, active, d) : "none"
    if (action !== "none") {
      var extra = root.sendEvening(action, now)
      for (var k in extra) changes[k] = extra[k]
      changes.lastAlertAt = now.getTime()
      if (action === "wind-down") changes.windDownDone = true
      else if (action === "bedtime") changes.bedtimeDone = true
      else changes.lateCount = (d.lateCount || 0) + 1
    }
    var mv = Model.moveAction(now.getTime(),
      { lastMovedAt: root.lastMovedAt, nudges: root.moveNudges, lastNudgeAt: root.lastNudgeAt },
      { everyMin: root.moveEvery, active: active, allowed: root.moveReminders && root.stepsFresh && root.moveWindow })
    if (mv === "nudge") {
      root.notify(strings.t("notify.move.title"),
                  strings.t("notify.move.body").arg(root.duration(root.sittingMinutes)).arg(Commons.I18n.formatNumber(root.moveSteps, 0)), false)
      root.moveNudges += 1
      root.lastNudgeAt = now.getTime()
    }
    if (d !== root.day || Object.keys(changes).length > 0) root.saveDay(Model.withChanges(d, changes))
  }

  function snooze() {
    root.saveDay(Model.withChanges(root.day, { snoozedUntil: Date.now() + 30 * 60000 }))
  }

  function toggleDoneForTonight() {
    root.saveDay(Model.withChanges(root.day, { doneForTonight: !root.day.doneForTonight }))
  }

  // Active = any input in the last five minutes, or an app holding an idle
  // inhibitor (a video playing late still costs sleep).
  IdleMonitor {
    id: activityMonitor
    timeout: 300
    respectInhibitors: true
  }

  FileView {
    id: dayFile
    path: root.stateDir + "/day.json"
    atomicWrites: true
    printErrors: false
    onLoaded: {
      root.day = Model.loadDay(text(), Model.dayKey(new Date()))
      root.dayLoaded = true
    }
    onLoadFailed: root.dayLoaded = true
  }

  // One sleep history per confirmed strap.
  readonly property string nightsPath: confirmedAddress !== ""
    ? root.stateDir + "/nights-" + confirmedAddress.replace(/:/g, "") + ".json" : ""
  onNightsPathChanged: root.nights = []

  FileView {
    id: nightsFile
    path: root.nightsPath
    atomicWrites: true
    printErrors: false
    onLoaded: root.nights = Model.mergeNights(Model.parseNightList(text()), root.nights, Date.now())
  }

  // Private to this account (0700), also when the folder already exists.
  Component.onCompleted: Quickshell.execDetached(["install", "-d", "-m", "700", root.stateDir])

  Timer {
    interval: 60000
    running: root.dayLoaded
    repeat: true
    onTriggered: root.tick()
  }

  // ---------- Views and movement ----------
  property string viewChoice: ""          // "" follows the time of day; set by the tabs
  property bool hrExpanded: false
  readonly property string view: viewChoice !== "" ? viewChoice : Model.autoView(pastBedtime)

  readonly property bool moveReminders: setting("moveReminders", true) !== false
  readonly property int moveEvery: Model.normalizeMoveEvery(setting("moveEvery", 50))
  readonly property int moveSteps: Model.normalizeMoveSteps(setting("moveSteps", 10))
  property var stepSamples: []
  property double lastMovedAt: Date.now()
  property bool movedSeen: false
  property int moveNudges: 0
  property double lastNudgeAt: 0
  property double lastStepsAt: 0
  property var hourly: null
  readonly property var hourBarsToday: Model.hourBars(hourly, steps, now)
  readonly property bool stepsFresh: lastStepsAt > 0 && now - lastStepsAt < 5 * 60000
  // No move nudges in the last hour before bed or while it's late.
  readonly property bool moveWindow: pastBedtime < -60 || pastBedtime >= Model.LATE_WINDOW_MINUTES
  readonly property int sittingMinutes: Model.clockSet(lastMovedAt) ? Math.max(0, Math.floor((now - lastMovedAt) / 60000)) : 0
  readonly property bool moveDue: moveReminders && stepsFresh && moveWindow && sittingMinutes >= moveEvery

  // ---------- Bar ----------
  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    // A moon from the wind-down hour; red (the bar's active color) once past bedtime.
    text: root.moonPhase !== "" ? "󰽥"
      : root.barMode === "steps" ? "󰖃"
      : root.linkState === "connected" ? "󰣐" : "󰋔"
    opacity: root.moonPhase !== "" ? 1
      : root.barMode === "steps" ? (root.steps >= 0 ? 1 : 0.55)
      : (root.live ? 1 : 0.55)
    active: root.moonPhase === "late" || (root.moonPhase === "" && root.moveDue)
      || (root.barMode !== "steps" && root.moonPhase === "" && root.zone >= 4)
    // The moon stands alone: no step count or heart rate while it shows.
    labelText: vertical || root.moonPhase !== "" ? ""
      : root.moveDue ? strings.t("bar.move") : root.barLabel
    slotSize: labelText === "" ? Style.bar.iconSlot
      : Style.bar.iconSlot + barLabelMetrics.advanceWidth + Style.space(8)
    tooltipText: root.opened ? ""
      : root.moonPhase === "late"
        ? strings.t("tooltip.moonLate").arg(root.duration(root.sleepIfNow)).arg(Model.formatClock(root.wakeTime))
      : root.moonPhase === "wind-down"
        ? strings.t("tooltip.moonWindDown").arg(Model.formatClock(root.bedtime))
      : root.moveDue
        ? strings.t("tooltip.move").arg(root.duration(root.sittingMinutes))
      : (root.live ? strings.t("tooltip.live").arg(root.bpmText).arg(root.zoneName(root.zone))
         : strings.t("tooltip." + root.linkState))
        + (root.steps >= 0 ? "\n" + strings.tn("tooltip.steps", root.steps).arg(root.stepsText) : "")
    onPressed: function(b) {
      if (b === Qt.RightButton) root.resetSession()
      else root.toggle()
    }
  }

  // ---------- Panel ----------
  KeyboardPanel {
    id: panel
    anchorItem: button
    owner: root
    bar: root.bar
    open: root.opened
    focusTarget: panel.loadedContent
    contentWidth: panel.fittedContentWidth(Style.space(380))
    contentHeight: panel.fittedContentHeight(panel.loadedContentHeight)

    content: PanelKeyCatcher {
      anchors.fill: parent
      implicitHeight: root.showSettings ? settingsPage.implicitHeight : column.implicitHeight
      onActivateRequested: if (!root.showSettings) root.resetSession()
      onCloseRequested: root.close()
      onTabRequested: function(direction) { root.switchPanel(direction) }

      Column {
        id: column
        visible: !root.showSettings
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        spacing: Style.space(14)

        // ---------- View tabs: movement by day, the sleep coach in the evening ----------
        ButtonGroup {
          width: parent.width
          value: root.view
          options: [
            { value: "move", label: strings.t("view.move") },
            { value: "sleep", label: strings.t("view.sleep") }
          ]
          foreground: root.foreground
          fontFamily: root.fontFamily
          fontSize: Style.font.bodySmall
          onChanged: function(v) { root.viewChoice = v }
        }

        // ---------- Move ----------
        Column {
          id: moveView
          visible: root.view === "move"
          width: parent.width
          spacing: Style.space(14)
          Column {
            visible: root.steps >= 0
            width: parent.width
            spacing: Style.space(6)

            Item {
              width: parent.width
              implicitHeight: Math.max(stepsValue.implicitHeight, stepsExtra.implicitHeight)

              Row {
                id: stepsValue
                spacing: Style.space(8)
                anchors.left: parent.left
                anchors.verticalCenter: parent.verticalCenter

                PlainText {
                  text: "󰖃"
                  font.pixelSize: Style.font.title
                  anchors.verticalCenter: parent.verticalCenter
                }
                PlainText {
                  text: root.stepsText
                  font.pixelSize: Style.font.display
                  font.bold: true
                }
                PlainText {
                  text: strings.tn("unit.steps", root.steps)
                  color: root.dim
                  font.pixelSize: Style.font.caption
                  anchors.baseline: parent.children[1].baseline
                }
              }

              PlainText {
                id: stepsExtra
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                color: root.dim
                font.pixelSize: Style.font.bodySmall
                text: [
                  root.distance >= 0 ? strings.t("activity.km").arg(Commons.I18n.formatNumber(root.distance / 1000, 2)) : "",
                  root.calories >= 0 ? strings.t("activity.kcal").arg(Commons.I18n.formatNumber(root.calories, 0)) : ""
                ].filter(function(t) { return t !== "" }).join("  ·  ")
              }
            }

            Rectangle {
              width: parent.width
              height: Style.space(6)
              radius: height / 2
              color: Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.1)

              Rectangle {
                height: parent.height
                radius: parent.radius
                width: parent.width * Model.goalProgress(root.steps, root.stepGoal)
                color: root.accent
                Behavior on width { NumberAnimation { duration: Style.duration(180); easing.type: Easing.OutCubic } }
              }
            }

            PlainText {
              text: root.steps >= root.stepGoal
                ? strings.t("activity.goalDone")
                : strings.t("activity.goal").arg(Commons.I18n.formatNumber(Math.round(Model.goalProgress(root.steps, root.stepGoal) * 100), 0))
                    .arg(Commons.I18n.formatNumber(root.stepGoal, 0))
              color: root.dim
              font.pixelSize: Style.font.caption
            }
          }

          // Sitting status: highlighted when it's time to move.
          PlainText {
            visible: root.steps >= 0 && root.moveReminders
            width: parent.width
            wrapMode: Text.Wrap
            font.pixelSize: Style.font.body
            font.bold: root.moveDue
            color: root.moveDue ? root.accent : root.dim
            text: root.moveDue
              ? strings.t("move.due").arg(root.duration(root.sittingMinutes)).arg(Commons.I18n.formatNumber(root.moveSteps, 0))
              : root.movedSeen ? strings.t("move.sitting").arg(root.duration(root.sittingMinutes))
              : strings.t("move.tracking")
          }
          PlainText {
            visible: root.steps < 0
            text: strings.t("steps." + (root.stepsState !== "" ? root.stepsState
                  : root.linkState === "connected" ? "authenticating" : "offline"))
            color: root.stepsState === "bad-key" || root.stepsState === "error" ? root.urgent : root.dim
            font.pixelSize: Style.font.bodySmall
            wrapMode: Text.Wrap
            width: parent.width
          }

          Button {
            visible: root.steps < 0 && root.needsSetup
            width: parent.width
            iconText: "󰌆"
            text: strings.t("action.setup")
            foreground: root.foreground
            fontFamily: root.fontFamily
            fontSize: Style.font.body
            bordered: true
            active: true
            verticalPadding: Style.spacing.controlPaddingY + Style.space(4)
            onClicked: root.setupSteps()
          }

          // Steps per hour today, 06:00 to midnight; the current hour highlighted.
          Column {
            visible: root.steps >= 0
            width: parent.width
            spacing: Style.space(4)

            PlainText {
              text: strings.t("move.perHour")
              color: root.dim
              font.pixelSize: Style.font.caption
            }

            Item {
              id: hourChart
              width: parent.width
              height: Style.space(56)
              readonly property var bars: root.hourBarsToday.slice(6)
              readonly property int peak: Math.max(250, Math.max.apply(null, bars))
              readonly property real gap: Style.space(2)

              Row {
                anchors.fill: parent
                spacing: hourChart.gap

                Repeater {
                  model: hourChart.bars.length

                  Item {
                    id: hourBar
                    required property int index
                    readonly property bool current: hourBar.index + 6 === root.nowDate.getHours()
                    width: (hourChart.width - hourChart.gap * (hourChart.bars.length - 1)) / hourChart.bars.length
                    height: hourChart.height

                    Rectangle {
                      anchors.bottom: parent.bottom
                      width: parent.width
                      height: Math.max(2, parent.height * hourChart.bars[hourBar.index] / hourChart.peak)
                      radius: Math.min(width / 2, Style.space(3))
                      color: hourBar.current ? root.accent : root.foreground
                      opacity: hourBar.current ? 1 : (hourChart.bars[hourBar.index] > 0 ? 0.45 : 0.12)
                    }
                  }
                }
              }
            }

            Item {
              width: parent.width
              height: Style.font.caption + Style.space(4)
              Repeater {
                model: [6, 9, 12, 15, 18, 21]
                PlainText {
                  required property int modelData
                  x: (modelData - 6) / 18 * parent.width
                  text: Model.pad2(modelData)
                  color: root.dim
                  font.pixelSize: Style.font.caption
                }
              }
            }
          }

          // Morning: a one-line recap of the night; click for the sleep coach.
          Button {
            visible: root.lastNight !== null && root.nowDate.getHours() < 12
            width: parent.width
            leftAlign: true
            iconText: "󰒲"
            text: root.lastNight ? strings.t("move.lastNight").arg(root.duration(root.lastNight.asleep)).arg(root.debtText()) : ""
            foreground: root.foreground
            fontFamily: root.fontFamily
            fontSize: Style.font.bodySmall
            bordered: true
            onClicked: root.viewChoice = "sleep"
          }
        }

        // ---------- Sleep coach ----------
        Column {
          id: sleepView
          visible: root.view === "sleep"
          width: parent.width
          spacing: Style.space(14)

          // Bedtime header: moon · countdown · clock (red once past it)
          Item {
            width: parent.width
            implicitHeight: Math.max(moonIcon.implicitHeight, bedLabels.implicitHeight, bedClock.implicitHeight)

            PlainText {
              id: moonIcon
              text: "󰽥"
              color: root.lateNow ? root.urgent : root.accent
              font.pixelSize: Style.font.display
              anchors.left: parent.left
              anchors.verticalCenter: parent.verticalCenter
            }

            Column {
              id: bedLabels
              anchors.left: moonIcon.right
              anchors.leftMargin: Style.space(14)
              anchors.right: bedClock.left
              anchors.rightMargin: Style.space(10)
              anchors.verticalCenter: parent.verticalCenter
              spacing: Style.space(2)

              PlainText {
                text: strings.t("tonight.bedtime")
                font.pixelSize: Style.font.title
                font.bold: true
              }
              PlainText {
                text: (root.lateNow ? strings.t("tonight.pastBy").arg(root.duration(root.pastBedtime))
                                    : strings.t("tonight.in").arg(root.duration(Model.minutesUntilBedtime(root.pastBedtime)))).toUpperCase()
                color: root.lateNow ? root.urgent : root.dim
                font.pixelSize: Style.font.caption
                font.bold: true
                font.letterSpacing: 1.2
              }
            }

            PlainText {
              id: bedClock
              text: Model.formatClock(root.bedtime)
              color: root.lateNow ? root.urgent : root.foreground
              font.pixelSize: Style.font.displayLarge
              font.bold: true
              anchors.right: parent.right
              anchors.verticalCenter: parent.verticalCenter
            }
          }
          Column {
            width: parent.width
            spacing: Style.space(6)

            PlainText {
              visible: root.lateNow
              width: parent.width
              wrapMode: Text.Wrap
              font.pixelSize: Style.font.body
              font.bold: root.lateNow
              color: root.lateNow && root.sleepIfNow < root.sleepNeed ? root.urgent : root.foreground
              text: root.lateNow
                ? strings.t("tonight.ifNow").arg(root.duration(root.sleepIfNow)).arg(Model.formatClock(root.wakeTime))
                : strings.t("tonight.until").arg(root.duration(Model.minutesUntilBedtime(root.pastBedtime))).arg(Model.formatClock(root.bedtime))
            }

            PlainText {
              width: parent.width
              wrapMode: Text.Wrap
              color: root.dim
              font.pixelSize: Style.font.bodySmall
              text: strings.t("tonight.plan").arg(Model.formatClock(root.bedtime))
                      .arg(Model.formatClock(root.wakeTime)).arg(root.duration(root.sleepNeed))
                    + (root.target.free ? "\n" + strings.t("tonight.free").arg(Model.formatClock(root.wakeTime)) : "")
                    + (root.plan.payback > 0 ? "\n" + strings.t("tonight.payback").arg(root.duration(root.plan.payback)) : "")
                    + (root.plan.waso >= 10 ? "\n" + strings.t("tonight.waso").arg(root.duration(root.plan.waso)) : "")
                    + (root.fallAsleepMeasured ? "\n" + strings.t("tonight.latency").arg(root.duration(root.fallAsleep)) : "")
            }

            PlainText {
              visible: root.wakeSchedule.errors.length > 0
              width: parent.width
              wrapMode: Text.Wrap
              color: root.urgent
              font.pixelSize: Style.font.caption
              text: strings.t("tonight.scheduleError").arg(root.wakeSchedule.errors.map(function(i) {
                return Qt.locale(Commons.I18n.formatLocale).dayName(i, Locale.LongFormat)
              }).join(", "))
            }

            PlainText {
              visible: root.sleepNeed < 420
              width: parent.width
              wrapMode: Text.Wrap
              color: root.urgent
              font.pixelSize: Style.font.caption
              text: strings.t("tonight.lowNeed")
            }

            PlainText {
              width: parent.width
              wrapMode: Text.Wrap
              color: root.dim
              font.pixelSize: Style.font.bodySmall
              text: root.day.firstActiveAt > 0
                ? strings.t("tonight.screen").arg(root.duration(root.day.activeMinutes))
                    .arg(new Date(root.day.firstActiveAt).toLocaleTimeString(Qt.locale(Commons.I18n.formatLocale), "HH:mm"))
                : strings.t("tonight.screenNone")
            }

            Row {
              visible: root.lateAlerts
              width: parent.width
              spacing: Style.space(6)

              Button {
                width: (parent.width - parent.spacing) / 2
                iconText: "󰒲"
                text: root.day.snoozedUntil > root.now
                  ? strings.t("action.snoozedUntil").arg(new Date(root.day.snoozedUntil).toLocaleTimeString(Qt.locale(Commons.I18n.formatLocale), "HH:mm"))
                  : strings.t("action.snooze")
                foreground: root.foreground
                fontFamily: root.fontFamily
                fontSize: Style.font.bodySmall
                bordered: true
                active: root.day.snoozedUntil > root.now
                enabled: !root.day.doneForTonight
                onClicked: root.snooze()
              }

              Button {
                width: (parent.width - parent.spacing) / 2
                iconText: root.day.doneForTonight ? "󰂚" : "󰂛"
                text: root.day.doneForTonight ? strings.t("action.alertsBack") : strings.t("action.doneTonight")
                foreground: root.foreground
                fontFamily: root.fontFamily
                fontSize: Style.font.bodySmall
                bordered: true
                active: root.day.doneForTonight
                onClicked: root.toggleDoneForTonight()
              }
            }
          }

          // ---------- Sleep ----------
          PanelSectionHeader {
            visible: root.nights.length > 0
            text: strings.t("section.sleep")
            foreground: root.foreground
            fontFamily: root.fontFamily
          }

          Column {
            visible: root.nights.length > 0
            width: parent.width
            spacing: Style.space(6)

            Item {
              visible: root.lastNight !== null
              width: parent.width
              implicitHeight: Math.max(sleepValue.implicitHeight, sleepTimes.implicitHeight)

              Row {
                id: sleepValue
                spacing: Style.space(8)
                anchors.left: parent.left
                anchors.verticalCenter: parent.verticalCenter

                PlainText {
                  text: "󰒲"
                  font.pixelSize: Style.font.title
                  anchors.verticalCenter: parent.verticalCenter
                }
                PlainText {
                  text: root.lastNight ? root.duration(root.lastNight.asleep) : ""
                  font.pixelSize: Style.font.display
                  font.bold: true
                  color: root.lastNight && root.lastNight.asleep < root.sleepNeed - 30 ? root.urgent : root.foreground
                }
              }

              PlainText {
                id: sleepTimes
                anchors.right: parent.right
                anchors.verticalCenter: parent.verticalCenter
                color: root.dim
                font.pixelSize: Style.font.bodySmall
                text: root.lastNight
                  ? strings.t("sleep.times").arg(root.clock(root.lastNight.start)).arg(root.clock(root.lastNight.end))
                      + (root.lastNight.score > 0 ? "  ·  " + strings.t("sleep.score").arg(root.lastNight.score) : "")
                  : ""
              }
            }

            // Stage proportions: deep · light · REM · awake
            Row {
              id: stageBar
              visible: root.lastNight !== null
              width: parent.width
              height: Style.space(6)
              spacing: Style.space(2)
              readonly property var parts: root.lastNight ? [
                { v: root.lastNight.deep, c: root.accent, o: 1 },
                { v: root.lastNight.light, c: root.accent, o: 0.45 },
                { v: root.lastNight.rem, c: root.foreground, o: 0.7 },
                { v: root.lastNight.awake, c: root.urgent, o: 0.8 }
              ] : []
              readonly property real total: parts.reduce(function(a, p) { return a + p.v }, 0)

              Repeater {
                model: stageBar.parts
                Rectangle {
                  required property var modelData
                  visible: modelData.v > 0
                  height: stageBar.height
                  radius: height / 2
                  width: stageBar.total > 0
                    ? Math.max(height, (stageBar.width - stageBar.spacing * 3) * modelData.v / stageBar.total) : 0
                  color: modelData.c
                  opacity: modelData.o
                }
              }
            }

            PlainText {
              visible: root.lastNight !== null
              width: parent.width
              wrapMode: Text.Wrap
              color: root.dim
              font.pixelSize: Style.font.caption
              text: root.lastNight ? strings.t("sleep.stages").arg(root.duration(root.lastNight.deep))
                      .arg(root.duration(root.lastNight.light)).arg(root.duration(root.lastNight.rem))
                      .arg(root.duration(root.lastNight.awake)) : ""
            }

            PlainText {
              width: parent.width
              wrapMode: Text.Wrap
              font.pixelSize: Style.font.bodySmall
              color: !root.debt.calibrating && root.debt.level === "high" ? root.urgent : root.foreground
              text: root.debtText()
            }

            PlainText {
              visible: text !== ""
              width: parent.width
              wrapMode: Text.Wrap
              color: root.dim
              font.pixelSize: Style.font.bodySmall
              text: root.regular.onsetSd === null ? ""
                : root.regular.irregular ? strings.t("regular.irregular").arg(root.duration(root.regular.onsetSd))
                : root.regular.jetlagFlag ? strings.t("regular.jetlag").arg(root.duration(root.regular.jetlag))
                : strings.t("regular.ok").arg(root.duration(root.regular.onsetSd))
            }
          }

          PlainText {
            visible: root.steps >= 0
            width: parent.width
            text: "󰖃  " + strings.tn("tooltip.steps", root.steps).arg(root.stepsText)
            color: root.dim
            font.pixelSize: Style.font.bodySmall
          }
        }

        // ---------- Heart rate (collapsed by default) ----------
        Button {
          width: parent.width
          leftAlign: true
          iconText: root.hrExpanded ? "󰅀" : "󰅂"
          text: strings.t("hr.section") + (root.live ? "  ·  " + root.bpmText + " " + strings.t("unit.bpm") : "")
          foreground: root.foreground
          fontFamily: root.fontFamily
          fontSize: Style.font.bodySmall
          onClicked: root.hrExpanded = !root.hrExpanded
        }

        Column {
          visible: root.hrExpanded
          width: parent.width
          spacing: Style.space(14)
          // ---------- Hero: heart · name/status · bpm ----------
          Item {
            width: parent.width
            implicitHeight: Math.max(heroIcon.implicitHeight, heroLabels.implicitHeight, heroBpm.implicitHeight)

            PlainText {
              id: heroIcon
              text: root.linkState === "connected" ? "󰣐" : "󰋔"
              color: root.live ? root.zoneColor(root.zone) : root.dim
              font.pixelSize: Style.font.display
              anchors.left: parent.left
              anchors.verticalCenter: parent.verticalCenter
            }

            Column {
              id: heroLabels
              anchors.left: heroIcon.right
              anchors.leftMargin: Style.space(14)
              anchors.right: heroBpm.left
              anchors.rightMargin: Style.space(10)
              anchors.verticalCenter: parent.verticalCenter
              spacing: Style.space(2)

              PlainText {
                text: root.deviceName !== "" ? root.deviceName : strings.t("title")
                font.pixelSize: Style.font.title
                font.bold: true
                elide: Text.ElideRight
                width: parent.width
              }

              PlainText {
                text: (root.live ? root.zoneName(root.zone)
                      : root.linkState === "connected" ? strings.t("status.waiting")
                      : strings.t("status." + root.linkState)).toUpperCase()
                color: root.live ? root.zoneColor(root.zone) : root.dim
                font.pixelSize: Style.font.caption
                font.bold: true
                font.letterSpacing: 1.2
                elide: Text.ElideRight
                width: parent.width
              }
            }

            Row {
              id: heroBpm
              anchors.right: parent.right
              anchors.verticalCenter: parent.verticalCenter
              spacing: Style.space(4)

              PlainText {
                text: root.bpmText
                font.pixelSize: Style.font.displayLarge
                font.bold: true
                opacity: root.live ? 1 : 0.5
              }
              PlainText {
                text: strings.t("unit.bpm")
                color: root.dim
                font.pixelSize: Style.font.caption
                anchors.baseline: parent.children[0].baseline
              }
            }
          }

          // ---------- Zone strip ----------
          Column {
            width: parent.width
            spacing: Style.space(4)

            Row {
              id: zoneRow
              width: parent.width
              spacing: Style.space(3)

              Repeater {
                model: 5

                Rectangle {
                  required property int index
                  readonly property int zoneNumber: index + 1
                  width: (zoneRow.width - zoneRow.spacing * 4) / 5
                  height: Style.space(6)
                  radius: height / 2
                  color: root.zoneColor(zoneNumber)
                  opacity: root.live && root.zone === zoneNumber ? 1 : 0.18
                  Behavior on opacity { NumberAnimation { duration: Style.duration(180); easing.type: Easing.OutCubic } }
                }
              }
            }

            Row {
              width: parent.width
              Repeater {
                model: 5
                PlainText {
                  required property int index
                  width: zoneRow.width / 5
                  horizontalAlignment: Text.AlignHCenter
                  text: "Z" + (index + 1) + "  " + Model.zoneBounds(index + 1, root.maxHr)[0] + "+"
                  color: root.dim
                  font.pixelSize: Style.font.caption
                }
              }
            }
          }

          // ---------- Ten-minute chart ----------
          Item {
            width: parent.width
            height: Style.space(84)

            Canvas {
              id: chart
              anchors.fill: parent
              readonly property var series: root.points
              readonly property color lineColor: root.accent
              readonly property color gridColor: root.foreground
              onSeriesChanged: requestPaint()
              onLineColorChanged: requestPaint()
              onWidthChanged: requestPaint()
              onPaint: {
                var ctx = getContext("2d")
                ctx.reset()
                var pts = series
                ctx.globalAlpha = 0.12
                ctx.strokeStyle = gridColor
                ctx.lineWidth = 1
                for (var g = 1; g < 4; g++) {
                  var gy = Math.round(height * g / 4) + 0.5
                  ctx.beginPath(); ctx.moveTo(0, gy); ctx.lineTo(width, gy); ctx.stroke()
                }
                ctx.globalAlpha = 1
                if (pts.length < 2) return
                var lo = Math.min.apply(null, pts), hi = Math.max.apply(null, pts)
                lo = Math.max(0, lo - 5); hi = hi + 5
                var step = width / (Model.MAX_POINTS - 1)
                var x0 = width - (pts.length - 1) * step
                function y(v) { return height - 2 - (v - lo) / (hi - lo) * (height - 4) }
                ctx.beginPath()
                ctx.moveTo(x0, y(pts[0]))
                for (var i = 1; i < pts.length; i++) ctx.lineTo(x0 + i * step, y(pts[i]))
                ctx.strokeStyle = lineColor
                ctx.lineWidth = 2
                ctx.lineJoin = "round"
                ctx.stroke()
                ctx.lineTo(x0 + (pts.length - 1) * step, height)
                ctx.lineTo(x0, height)
                ctx.closePath()
                ctx.globalAlpha = 0.12
                ctx.fillStyle = lineColor
                ctx.fill()
              }
            }

            PlainText {
              anchors.centerIn: parent
              visible: root.points.length < 2
              text: strings.t("chart.empty")
              color: root.dim
              font.pixelSize: Style.font.bodySmall
            }

            PlainText {
              anchors.left: parent.left
              anchors.bottom: parent.bottom
              visible: root.points.length >= 2
              text: strings.t("chart.span")
              color: root.dim
              font.pixelSize: Style.font.caption
            }
          }

          // ---------- Session stats ----------
          Row {
            id: statRow
            width: parent.width
            readonly property var items: [
              { key: "stat.min", value: root.stats.count ? Commons.I18n.formatNumber(root.stats.min, 0) : "--" },
              { key: "stat.avg", value: root.stats.count ? Commons.I18n.formatNumber(Model.average(root.stats), 0) : "--" },
              { key: "stat.max", value: root.stats.count ? Commons.I18n.formatNumber(root.stats.max, 0) : "--" },
              { key: root.hrv > 0 ? "stat.hrv" : "stat.session",
                value: root.hrv > 0 ? Commons.I18n.formatNumber(root.hrv, 0) : Model.formatDuration((root.now - root.sessionStartedAt) / 1000) }
            ]

            Repeater {
              model: statRow.items

              Column {
                required property var modelData
                width: statRow.width / 4
                spacing: Style.space(2)

                PlainText {
                  width: parent.width
                  horizontalAlignment: Text.AlignHCenter
                  text: modelData.value
                  font.pixelSize: Style.font.title
                  font.bold: true
                }
                PlainText {
                  width: parent.width
                  horizontalAlignment: Text.AlignHCenter
                  text: strings.t(modelData.key).toUpperCase()
                  color: root.dim
                  font.pixelSize: Style.font.caption
                  font.letterSpacing: 1
                }
              }
            }
          }

          // ---------- Time in zones ----------
          PanelSectionHeader {
            text: strings.t("section.zones")
            foreground: root.foreground
            fontFamily: root.fontFamily
          }

          Column {
            width: parent.width
            spacing: Style.space(4)

            Repeater {
              model: [5, 4, 3, 2, 1]

              Item {
                id: zoneLine
                required property int modelData
                readonly property real secs: root.stats.zoneSeconds[modelData]
                readonly property real total: {
                  var t = 0
                  for (var i = 0; i < root.stats.zoneSeconds.length; i++) t += root.stats.zoneSeconds[i]
                  return t
                }
                width: parent.width
                height: Math.max(zoneLabel.implicitHeight, Style.space(14))

                PlainText {
                  id: zoneLabel
                  width: Style.space(110)
                  anchors.verticalCenter: parent.verticalCenter
                  text: "Z" + zoneLine.modelData + "  " + root.zoneName(zoneLine.modelData)
                  font.pixelSize: Style.font.bodySmall
                  elide: Text.ElideRight
                }

                Rectangle {
                  id: track
                  anchors.left: zoneLabel.right
                  anchors.right: zoneTime.left
                  anchors.rightMargin: Style.space(8)
                  anchors.verticalCenter: parent.verticalCenter
                  height: Style.space(6)
                  radius: height / 2
                  color: Qt.rgba(root.foreground.r, root.foreground.g, root.foreground.b, 0.1)

                  Rectangle {
                    height: parent.height
                    radius: parent.radius
                    width: zoneLine.total > 0 ? parent.width * zoneLine.secs / zoneLine.total : 0
                    color: root.zoneColor(zoneLine.modelData)
                  }
                }

                PlainText {
                  id: zoneTime
                  width: Style.space(44)
                  anchors.right: parent.right
                  anchors.verticalCenter: parent.verticalCenter
                  horizontalAlignment: Text.AlignRight
                  text: Model.formatDuration(zoneLine.secs)
                  color: root.dim
                  font.pixelSize: Style.font.bodySmall
                }
              }
            }
          }
        }

        PanelSeparator { foreground: root.foreground }

        // ---------- Device ----------
        Row {
          width: parent.width
          spacing: Style.space(16)

          PlainText {
            text: Model.batteryIcon(root.battery) + "  " + (root.battery >= 0
              ? strings.t("device.battery").arg(Commons.I18n.formatNumber(root.battery, 0)) : "--")
            color: root.battery >= 0 && root.battery <= 15 ? root.urgent : root.foreground
            font.pixelSize: Style.font.bodySmall
          }
          PlainText {
            visible: root.firmware !== ""
            text: strings.t("device.firmware").arg(root.firmware)
            color: root.dim
            font.pixelSize: Style.font.bodySmall
          }
        }

        // Which strap is yours? Nothing is read until one is confirmed.
        Column {
          visible: !root.helperMissing && !root.helperOutdated && root.confirmedAddress === ""
          width: parent.width
          spacing: Style.space(8)

          PlainText {
            width: parent.width
            text: strings.t("confirm.title")
            font.pixelSize: Style.font.body
            font.bold: true
          }
          PlainText {
            width: parent.width
            wrapMode: Text.Wrap
            text: root.candidates.length > 0 ? strings.t("confirm.body") : strings.t("confirm.none")
            color: root.dim
            font.pixelSize: Style.font.bodySmall
          }
          Repeater {
            model: root.candidates
            Button {
              required property var modelData
              width: parent.width
              iconText: "󰂯"
              text: strings.t("confirm.choose").arg(modelData.name !== "" ? modelData.name : strings.t("title")).arg(modelData.address)
              foreground: root.foreground
              fontFamily: root.fontFamily
              fontSize: Style.font.bodySmall
              bordered: true
              onClicked: root.confirmStrap(modelData.address)
            }
          }
        }

        PlainText {
          visible: root.helperMissing || root.helperOutdated
          text: root.helperOutdated ? strings.t("helper.outdated").arg(root.helperVersion) : strings.t("helper.missing")
          color: root.urgent
          font.pixelSize: Style.font.bodySmall
          wrapMode: Text.Wrap
          width: parent.width
        }
        Button {
          visible: root.helperMissing || root.helperOutdated
          width: parent.width
          iconText: "󰏔"
          text: strings.t(root.helperOutdated ? "action.updateHelper" : "action.installHelper")
          foreground: root.foreground
          fontFamily: root.fontFamily
          fontSize: Style.font.body
          bordered: true
          active: true
          verticalPadding: Style.spacing.controlPaddingY + Style.space(4)
          onClicked: root.installHelper()
        }

        PlainText {
          visible: !root.helperMissing && !root.helperOutdated && root.linkState === "searching" && root.helperError !== ""
          text: root.helperError
          color: root.urgent
          font.pixelSize: Style.font.caption
          wrapMode: Text.Wrap
          width: parent.width
        }

        Row {
          width: parent.width
          spacing: Style.space(6)

          Button {
            width: (parent.width - parent.spacing * 2) / 3
            iconText: "󰑓"
            text: strings.t("action.reset")
            foreground: root.foreground
            fontFamily: root.fontFamily
            fontSize: Style.font.bodySmall
            bordered: true
            onClicked: root.resetSession()
          }

          Button {
            width: (parent.width - parent.spacing * 2) / 3
            iconText: "󰂯"
            text: strings.t("action.bluetooth")
            foreground: root.foreground
            fontFamily: root.fontFamily
            fontSize: Style.font.bodySmall
            bordered: true
            onClicked: root.openBluetooth()
          }

          Button {
            width: (parent.width - parent.spacing * 2) / 3
            iconText: "󰒓"
            text: strings.t("action.settings")
            foreground: root.foreground
            fontFamily: root.fontFamily
            fontSize: Style.font.bodySmall
            bordered: true
            onClicked: root.showSettings = true
          }
        }
      }

      // ---------- Settings page ----------
      Column {
        id: settingsPage
        visible: root.showSettings
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        spacing: Style.space(10)

        Item {
          width: parent.width
          implicitHeight: backButton.implicitHeight

          Button {
            id: backButton
            anchors.left: parent.left
            iconText: Commons.I18n.rightToLeft ? "󰅂" : "󰅁"
            text: strings.t("action.back")
            foreground: root.foreground
            fontFamily: root.fontFamily
            fontSize: Style.font.bodySmall
            onClicked: root.showSettings = false
          }

          PlainText {
            anchors.centerIn: parent
            text: strings.t("settings.title")
            font.pixelSize: Style.font.title
            font.bold: true
          }
        }

        // -- Tabs: the page is too tall for one column
        ButtonGroup {
          width: parent.width
          value: root.settingsTab
          options: [
            { value: "sleep", label: strings.t("settings.tab.sleep") },
            { value: "activity", label: strings.t("settings.tab.activity") },
            { value: "strap", label: strings.t("settings.tab.strap") },
            { value: "about", label: strings.t("settings.tab.about") }
          ]
          foreground: root.foreground
          fontFamily: root.fontFamily
          fontSize: Style.font.bodySmall
          onChanged: function(v) { root.settingsTab = v }
        }

        Column {
          width: parent.width
          spacing: Style.space(10)
          visible: root.settingsTab === "sleep"

          // -- Sleep

          PlainText {
            width: parent.width
            wrapMode: Text.Wrap
            color: root.dim
            font.pixelSize: Style.font.caption
            text: strings.t("settings.wakeHint")
          }

          // Monday first; Model.WAKE_KEYS is indexed Sunday..Saturday.
          Repeater {
            model: [1, 2, 3, 4, 5, 6, 0]

            SettingRow {
              id: wakeRow
              required property int modelData
              readonly property string key: Model.WAKE_KEYS[modelData]
              readonly property string raw: root.wakeValue(key)
              readonly property bool free: /^(free|libre|off|-)$/i.test(raw.trim())
              readonly property string shown: Model.parseClock(raw) !== null ? Model.formatClock(Model.parseClock(raw)) : "07:00"
              label: Qt.locale(Commons.I18n.formatLocale).dayName(modelData, Locale.LongFormat)

              TextField {
                id: wakeField
                width: Style.space(72)
                enabled: !wakeRow.free
                horizontalAlignment: TextInput.AlignHCenter
                text: wakeRow.free ? "" : wakeRow.shown
                placeholderText: wakeRow.free ? strings.t("settings.free") : "07:00"
                foreground: root.foreground
                font.family: root.fontFamily
                font.pixelSize: Style.font.bodySmall
                maximumLength: 5
                onEditingFinished: {
                  var t = Model.parseClock(text)
                  if (t === null) { text = wakeRow.shown; return }
                  if (Model.formatClock(t) !== wakeRow.raw) root.setSetting(wakeRow.key, Model.formatClock(t))
                }
              }

              PlainText {
                anchors.verticalCenter: parent.verticalCenter
                text: strings.t("settings.free")
                color: root.dim
                font.pixelSize: Style.font.caption
              }

              ToggleSwitch {
                anchors.verticalCenter: parent.verticalCenter
                checked: wakeRow.free
                foreground: root.foreground
                onToggled: root.setSetting(wakeRow.key, wakeRow.free ? wakeRow.shown : "free")
              }
            }
          }

          SettingRow {
            label: strings.t("settings.sleepNeed")

            Dropdown {
              width: Style.space(110)
              showLabel: false
              value: String(root.setting("sleepNeed", "8h"))
              options: Model.SLEEP_NEEDS
              foreground: root.foreground
              fontFamily: root.fontFamily
              onChanged: function(v) { root.setSetting("sleepNeed", v) }
            }
          }

          SettingRow {
            label: strings.t("settings.fallAsleep")

            PlainText {
              anchors.verticalCenter: parent.verticalCenter
              text: strings.t("settings.auto")
              color: root.dim
              font.pixelSize: Style.font.caption
            }

            ToggleSwitch {
              anchors.verticalCenter: parent.verticalCenter
              checked: root.fallAsleepAuto
              foreground: root.foreground
              onToggled: root.setSetting("fallAsleepAuto", !root.fallAsleepAuto)
            }

            NumberField {
              value: root.fallAsleepManual
              enabled: !root.fallAsleepMeasured
              opacity: enabled ? 1 : 0.5
              from: 0
              to: 90
              stepSize: 5
              foreground: root.foreground
              fontFamily: root.fontFamily
              onModified: function(v) { root.setSetting("fallAsleep", v) }
            }
          }

          PlainText {
            visible: root.fallAsleepAuto
            width: parent.width
            wrapMode: Text.Wrap
            color: root.dim
            font.pixelSize: Style.font.caption
            text: root.fallAsleepMeasured
              ? strings.t("settings.latencyMeasured").arg(root.duration(root.latencyInfo.minutes)).arg(root.latencyInfo.nights)
              : strings.t("settings.latencyLearning").arg(root.latencyInfo.nights).arg(5).arg(root.duration(root.fallAsleepManual))
          }

          SettingRow {
            label: strings.t("settings.reminders")

            ToggleSwitch {
              checked: root.lateAlerts
              foreground: root.foreground
              onToggled: root.setSetting("lateAlerts", !root.lateAlerts)
            }
          }
        }

        Column {
          width: parent.width
          spacing: Style.space(10)
          visible: root.settingsTab === "activity"

          // -- Movement
          PanelSectionHeader {
            text: strings.t("settings.movement")
            foreground: root.foreground
            fontFamily: root.fontFamily
          }

          SettingRow {
            label: strings.t("settings.moveReminders")

            ToggleSwitch {
              checked: root.moveReminders
              foreground: root.foreground
              onToggled: root.setSetting("moveReminders", !root.moveReminders)
            }
          }

          SettingRow {
            label: strings.t("settings.moveEvery")

            NumberField {
              value: root.moveEvery
              from: 20
              to: 180
              stepSize: 5
              foreground: root.foreground
              fontFamily: root.fontFamily
              onModified: function(v) { root.setSetting("moveEvery", v) }
            }
          }

          SettingRow {
            label: strings.t("settings.moveSteps")

            NumberField {
              value: root.moveSteps
              from: 10
              to: 1000
              stepSize: 10
              foreground: root.foreground
              fontFamily: root.fontFamily
              onModified: function(v) { root.setSetting("moveSteps", v) }
            }
          }

          // -- Activity and bar
          PanelSectionHeader {
            text: strings.t("settings.activity")
            foreground: root.foreground
            fontFamily: root.fontFamily
          }

          SettingRow {
            label: strings.t("settings.stepGoal")

            NumberField {
              value: root.stepGoal
              from: 500
              to: 100000
              stepSize: 500
              foreground: root.foreground
              fontFamily: root.fontFamily
              onModified: function(v) { root.setSetting("stepGoal", v) }
            }
          }

          SettingRow {
            label: strings.t("settings.maxHr")

            NumberField {
              value: root.maxHr
              from: 100
              to: 230
              stepSize: 1
              foreground: root.foreground
              fontFamily: root.fontFamily
              onModified: function(v) { root.setSetting("maxHeartRate", v) }
            }
          }

          PlainText {
            width: parent.width
            text: strings.t("settings.barShows")
            font.pixelSize: Style.font.bodySmall
          }

          ButtonGroup {
            width: parent.width
            value: String(root.setting("barShows", "Heart rate and steps"))
            options: [
              { value: "Heart rate and steps", label: strings.t("settings.bar.both") },
              { value: "Heart rate", label: strings.t("settings.bar.hr") },
              { value: "Steps", label: strings.t("settings.bar.steps") }
            ]
            foreground: root.foreground
            fontFamily: root.fontFamily
            fontSize: Style.font.bodySmall
            onChanged: function(v) { root.setSetting("barShows", v) }
          }

          SettingRow {
            label: strings.t("settings.zoneInBar")

            ToggleSwitch {
              checked: root.showZoneInBar
              foreground: root.foreground
              onToggled: root.setSetting("showZoneInBar", !root.showZoneInBar)
            }
          }
        }

        Column {
          width: parent.width
          spacing: Style.space(10)
          visible: root.settingsTab === "strap"

          // -- Strap

          SettingRow {
            label: root.confirmedAddress !== "" ? strings.t("settings.confirmed").arg(root.confirmedAddress) : strings.t("settings.noConfirmed")

            Button {
              visible: root.confirmedAddress !== ""
              text: strings.t("action.forgetStrap")
              foreground: root.foreground
              fontFamily: root.fontFamily
              fontSize: Style.font.bodySmall
              bordered: true
              onClicked: root.forgetStrap()
            }
          }

          SettingRow {
            label: strings.t("settings.deviceName")

            TextField {
              width: Style.space(140)
              text: root.deviceFilter
              placeholderText: strings.t("settings.anyStrap")
              foreground: root.foreground
              font.family: root.fontFamily
              font.pixelSize: Style.font.bodySmall
              maximumLength: 64
              onEditingFinished: if (text.trim() !== root.deviceFilter) root.setSetting("deviceName", text.trim())
            }
          }

          SettingRow {
            label: strings.t("settings.reconnect")

            ToggleSwitch {
              checked: root.autoReconnect
              foreground: root.foreground
              onToggled: root.setSetting("autoReconnect", !root.autoReconnect)
            }
          }

          Button {
            width: parent.width
            iconText: "󰌆"
            text: strings.t("action.setup")
            foreground: root.foreground
            fontFamily: root.fontFamily
            fontSize: Style.font.bodySmall
            bordered: true
            onClicked: root.setupSteps()
          }
        }

        Column {
          width: parent.width
          spacing: Style.space(10)
          visible: root.settingsTab === "about"

          // -- About

          SettingRow {
            label: strings.t("settings.version").arg(root.pluginVersion)

            Button {
              iconText: "󰎈"
              text: strings.t("action.intro")
              foreground: root.foreground
              fontFamily: root.fontFamily
              fontSize: Style.font.bodySmall
              bordered: true
              onClicked: root.playIntro()
            }
          }
        }
      }
    }

  }

  // A label on the left, its controls on the right.
  component SettingRow: Item {
    id: settingRow
    property string label: ""
    default property alias controls: holder.data
    width: parent ? parent.width : 0
    implicitHeight: Math.max(rowLabel.implicitHeight, holder.implicitHeight)

    Text {
      id: rowLabel
      textFormat: Text.PlainText
      anchors.left: parent.left
      anchors.right: holder.left
      anchors.rightMargin: Style.space(8)
      anchors.verticalCenter: parent.verticalCenter
      text: settingRow.label
      color: root.foreground
      font.family: root.fontFamily
      font.pixelSize: Style.font.bodySmall
      elide: Text.ElideRight
    }

    Row {
      id: holder
      anchors.right: parent.right
      anchors.verticalCenter: parent.verticalCenter
      spacing: Style.space(8)
    }
  }

  component PlainText: Text {
    textFormat: Text.PlainText
    color: root.foreground
    font.family: root.fontFamily
  }
}
