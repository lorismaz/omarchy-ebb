import QtQuick
import QtMultimedia
import "banner.js" as Banner

// Ebb's intro, in Omarchy's own visual language: the name in the block letters
// of the Omarchy logo, decrypted like the screensaver's text effects and crossed
// by the About screen's 45-degree sheen; under it a tide that flows in (the day,
// moving) and ebbs out (the night, resting) in time with the music, a ripple
// running across it on every heartbeat; then a terminal line typing the
// credits. Theme colours in, nothing else. Any key or click ends it.
Item {
  id: root

  property var rows: Banner.ROWS
  property string version: "1.3.0"
  property var lines: [
    "ebb-strap --version  " + version,
    "flow by day: steps and move reminders, proven by your feet",
    "ebb by night: sleep debt, naps and a coach at bedtime",
    "for the amazfit helio strap, in your omarchy bar",
    "rust + qml. the music is pure math",
    "greetings to the omarchy crew and the gadgetbridge hackers",
    "press any key"
  ]
  property string flowLabel: "flow  day, move"
  property string ebbLabel: "ebb  night, rest"
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
    readonly property int tideRows: 6
    readonly property int span: 60              // grid width in cells: the tide and the terminal line

    property var cells: []          // banner cells: {r, c, ch, appear, reveal}
    property int cols: 0

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
    }

    onPaint: {
      const ctx = getContext("2d")
      if (cells.length === 0) build()
      const t = root.t
      const music = player.playbackState === MediaPlayer.PlayingState ? player.position / 1000 : t
      const beat = music * root.bpm / 60

      // Grid: banner, gap, tide, gap, terminal line.
      const total = root.rows.length + 2 + tideRows + 3
      let ch = Math.floor(Math.min(height / (total + 6), width / (span + 8) * 2))
      ch -= ch % 2
      const cw = ch / 2
      const gx = Math.round((width - span * cw) / 2)                 // grid left
      const x0 = gx + Math.round((span - cols) / 2) * cw              // banner left
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

      // ---- The tide: flows in over eight beats, ebbs out over the next eight.
      const ty0 = y0 + (root.rows.length + 2) * ch
      const levels = tideRows * 2
      const phase = beat / 16 * 2 * Math.PI
      const rising = Math.sin(phase) >= 0
      const fill = Math.min(1, t / decryptEnd)                         // the water comes in with the logo
      const level = (0.5 - 0.3 * Math.cos(phase)) * fill
      const pulse = (beat / 2) % 1                                    // a heartbeat every two beats
      const at = pulse * span * 1.3 - span * 0.15
      for (let c = 0; c < span; c++) {
        const ripple = 0.16 * Math.exp(-((c - at) * (c - at)) / 6) * (1 - pulse)
        const swell = 0.09 * Math.sin(c * 0.33 - music * 2.4) + 0.05 * Math.sin(c * 0.9 + music * 3.7)
        const n = Math.max(0, Math.min(levels, Math.round((level + (swell + ripple) * fill) * levels)))
        for (let l = 0; l < n; l++) {
          const crest = l === n - 1
          ctx.fillStyle = crest && ripple > 0.05 ? "#ffffff"
                        : crest ? root.foreground
                        : hex(root.accent, 0.25 + 0.6 * (l + 1) / levels)
          ctx.fillRect(gx + c * cw, ty0 + (levels - 1 - l) * ch / 2, cw, ch / 2)
        }
      }
      ctx.fillStyle = hex(root.muted, 0.35)                           // the shore line
      ctx.fillRect(gx, ty0 + tideRows * ch + ch / 4, span * cw, 1)
      if (t > decryptEnd) {
        ctx.font = Math.round(ch * 0.7) + "px \"" + root.fontFamily + "\""
        ctx.textAlign = "right"
        ctx.fillStyle = hex(root.muted, 0.9)
        ctx.fillText((rising ? "↑ " : "↓ ") + (rising ? root.flowLabel : root.ebbLabel), gx + span * cw, ty0 - ch * 0.4)
      }

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
        const ty = y0 + (root.rows.length + 2 + tideRows + 2) * ch + ch / 2
        ctx.font = Math.round(ch * 0.85) + "px \"" + root.fontFamily + "\""
        ctx.textAlign = "left"
        const prompt = "❯ "
        ctx.fillStyle = root.accent
        ctx.fillText(prompt, gx, ty)
        const px = gx + ctx.measureText(prompt).width
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
