import QtQuick
import QtMultimedia
import "banner.js" as Banner

// An intro in Omarchy's own visual language: the name in the block letters of
// the Omarchy logo, decrypted like the screensaver's text effects, crossed by
// the 45-degree sheen of the About screen on every other bar, a heart trace
// beating on the music, and a terminal line typing the credits. Theme colours
// in, nothing else. Any key or click ends it.
Item {
  id: root

  property var rows: Banner.ROWS
  property string version: "1.2.1"
  property var lines: [
    "helio-strap --version  " + version,
    "steps and move reminders, proven by your feet",
    "sleep, sleep debt and a coach at bedtime",
    "rust + qml. the music is pure math",
    "greetings to the omarchy crew and the gadgetbridge hackers",
    "press any key"
  ]
  property url music: Qt.resolvedUrl("intro.ogg")
  property real bpm: 140
  property bool sound: true

  // Omarchy theme tokens; the panel passes the live ones.
  property color background: "#101315"
  property color foreground: "#cacccc"
  property color accent: "#cacccc"
  property color muted: "#707880"
  property color urgent: "#a55555"
  property string fontFamily: "monospace"

  signal finished()

  property real t: 0

  focus: true
  Keys.onPressed: (e) => { e.accepted = true; root.finished() }

  Rectangle { anchors.fill: parent; color: root.background }

  MediaPlayer {
    id: player
    source: root.music
    loops: MediaPlayer.Infinite
    audioOutput: AudioOutput { volume: root.sound ? 0.8 : 0 }
    Component.onCompleted: play()
  }

  FrameAnimation {
    running: true
    onTriggered: { root.t += frameTime; screen.requestPaint() }
  }

  Canvas {
    id: screen
    anchors.fill: parent
    renderStrategy: Canvas.Cooperative

    readonly property string scramble: "░▒▓▖▗▘▙▚▛▜▝▞▟#%&*+=<>/\\"
    readonly property real decryptEnd: 2.6
    readonly property int ecgRows: 4

    property var cells: []          // banner cells: {r, c, ch, appear, reveal}
    property int cols: 0
    property var trace: []          // per column: {v, at}
    property real head: 0
    property real lastT: 0

    function hex(c, a) { return Qt.rgba(c.r, c.g, c.b, a === undefined ? 1 : a) }

    function build() {
      cells = []
      cols = 0
      for (let r = 0; r < root.rows.length; r++) {
        const line = root.rows[r]
        cols = Math.max(cols, line.length)
        for (let c = 0; c < line.length; c++) {
          if (line[c] === " ") continue
          cells.push({ r: r, c: c, ch: line[c], appear: Math.random() * 0.8,
                       reveal: 0.7 + c / 70 * 1.2 + Math.random() * 0.7 })
        }
      }
      trace = []
      for (let i = 0; i < cols; i++) trace.push(null)
    }

    // One heartbeat, phase 0..1: P wave, QRS spike, T wave. About -0.3..1
    function ecg(p) {
      const bump = (c, w, h) => h * Math.exp(-((p - c) * (p - c)) / (2 * w * w))
      return bump(0.12, 0.025, 0.12) - bump(0.27, 0.008, 0.12) + bump(0.30, 0.010, 1.0)
           - bump(0.33, 0.010, 0.28) + bump(0.55, 0.05, 0.25)
    }

    onPaint: {
      const ctx = getContext("2d")
      if (cells.length === 0) build()
      const t = root.t
      const music = player.playbackState === MediaPlayer.PlayingState ? player.position / 1000 : t
      const beat = music * root.bpm / 60

      // Grid: banner, gap, heart trace, gap, terminal line.
      const total = root.rows.length + 2 + ecgRows + 3
      let ch = Math.floor(Math.min(height / (total + 6), width / (cols + 8) * 2))
      ch -= ch % 2
      const cw = ch / 2
      const x0 = Math.round((width - cols * cw) / 2)
      const y0 = Math.round((height - total * ch) / 2)

      ctx.fillStyle = root.background
      ctx.fillRect(0, 0, width, height)

      // ---- Banner: decrypt, then the sheen on every other bar.
      const sweepBeat = beat % 8
      const sweeping = t > decryptEnd && sweepBeat < 3
      const centre = sweepBeat / 3 * (cols + root.rows.length * 2 + 10) - 5
      ctx.font = Math.round(ch * 0.8) + "px \"" + root.fontFamily + "\""
      ctx.textAlign = "center"
      ctx.textBaseline = "middle"
      for (const cell of cells) {
        const x = x0 + cell.c * cw, y = y0 + cell.r * ch
        if (t < cell.appear) continue
        if (t < cell.reveal) {
          const k = Math.floor(t * 16 + cell.c * 7 + cell.r * 13) % scramble.length
          ctx.fillStyle = hex(root.muted, 0.9)
          ctx.fillText(scramble[k], x + cw / 2, y + ch / 2)
          continue
        }
        const fresh = t - cell.reveal < 0.12                         // flashes white as it lands
        const inBand = sweeping && Math.abs(cell.c - (centre - cell.r * 2)) <= 2
        ctx.fillStyle = inBand || fresh ? "#ffffff" : root.accent
        if (cell.ch === "▄") ctx.fillRect(x, y + ch / 2, cw, ch / 2)
        else if (cell.ch === "▀") ctx.fillRect(x, y, cw, ch / 2)
        else ctx.fillRect(x, y, cw, ch)
      }

      // ---- Heart trace: a monitor sweep, one heartbeat every two beats.
      const ey = y0 + (root.rows.length + 2) * ch
      const levels = ecgRows * 2
      const speed = cols / (6 * 60 / root.bpm)                          // three heartbeats a sweep
      if (t > 0.6) {
        const dt = Math.min(0.1, t - lastT)
        const from = head
        head = (head + dt * speed) % cols
        const steps = Math.ceil(dt * speed) + 1
        for (let s = 0; s < steps; s++) {
          const col = (Math.floor(from) + s) % cols
          const ago = (steps - 1 - s) / speed
          // Keep the strongest sample across the column, so the narrow spike is never missed.
          let v = 0
          for (let k = 0; k < 6; k++) {
            const p = ((music - ago - k / 6 / speed) * root.bpm / 60 / 2 % 1 + 1) % 1
            const e = ecg(p)
            if (Math.abs(e) > Math.abs(v)) v = e
          }
          trace[col] = { v: v, at: t - ago }
        }
      }
      lastT = t
      const life = cols / speed
      let prev = null
      for (let c = 0; c < cols; c++) {
        const s = trace[c]
        const age = s ? t - s.at : life
        if (age >= life) { prev = null; continue }
        const a = 1 - age / life
        const lv = Math.max(0, Math.min(levels - 1, Math.round((s.v + 0.3) / 1.3 * (levels - 1))))
        const lo = prev === null ? lv : Math.min(lv, prev), hi = prev === null ? lv : Math.max(lv, prev)
        ctx.fillStyle = c === Math.floor(head) ? "#ffffff" : hex(root.urgent, 0.2 + 0.8 * a)
        for (let l = lo; l <= hi; l++)
          ctx.fillRect(x0 + c * cw, ey + (levels - 1 - l) * ch / 2, cw, ch / 2)
        prev = lv
      }
      ctx.fillStyle = hex(root.muted, 0.35)
      for (let c = 0; c < cols; c += 2) ctx.fillRect(x0 + c * cw + cw / 2 - 1, ey + ecgRows * ch + ch / 4, 2, 2)

      // ---- Terminal line typing the credits.
      if (t > decryptEnd) {
        const typeRate = 32, eraseRate = 90, hold = 2.2
        const durations = root.lines.map(l => l.length / typeRate + hold + l.length / eraseRate + 0.3)
        let local = (t - decryptEnd) % durations.reduce((a, b) => a + b, 0)
        let n = 0
        while (n < durations.length - 1 && local > durations[n]) { local -= durations[n]; n++ }
        const line = root.lines[n]
        const typed = line.length / typeRate
        const shown = local < typed ? Math.floor(local * typeRate)
                    : local < typed + hold ? line.length
                    : Math.max(0, line.length - Math.floor((local - typed - hold) * eraseRate))
        const ty = y0 + (root.rows.length + 2 + ecgRows + 2) * ch + ch / 2
        ctx.font = Math.round(ch * 0.85) + "px \"" + root.fontFamily + "\""
        ctx.textAlign = "left"
        const prompt = "❯ "
        ctx.fillStyle = root.accent
        ctx.fillText(prompt, x0, ty)
        const px = x0 + ctx.measureText(prompt).width
        const text = line.slice(0, shown)
        ctx.fillStyle = root.foreground
        ctx.fillText(text, px, ty)
        if (Math.floor(t * 2) % 2 === 0 || local < typed)
          ctx.fillRect(px + ctx.measureText(text).width + 1, ty - ch * 0.42, cw * 0.9, ch * 0.84)
      }

      // Fade in from the background.
      if (t < 0.5) {
        ctx.fillStyle = hex(root.background, 1 - t / 0.5)
        ctx.fillRect(0, 0, width, height)
      }
    }
  }

  MouseArea { anchors.fill: parent; onClicked: root.finished() }
}
