#!/usr/bin/env python3
"""Original keygen-style chiptune for the Ebb intro.

Pure Python, no dependencies: tracker-style chord arpeggios on a pulse wave,
an octave bass, a lead with vibrato and noise drums, 16 bars in A minor that
loop seamlessly. Writes a 16-bit mono WAV; encode it with
    ffmpeg -i intro.wav -c:a libvorbis -q:a 3 intro.ogg

Usage: python3 chiptune.py out.wav
"""

import math
import random
import struct
import sys
import wave

RATE = 22050
BPM = 140
STEP = 60 / BPM / 4                  # one 16th note, seconds
TICK = 1 / 50                        # tracker arpeggio speed (PAL tick)
BARS = 16
TOTAL = int(BARS * 16 * STEP * RATE)

# Am F C G, as (root, third, fifth) MIDI notes
CHORDS = [(57, 60, 64), (53, 57, 60), (48, 52, 55), (55, 59, 62)]

# Lead: per bar, (step, midi, length in steps)
PHRASE = [
    [(0, 76, 3), (3, 74, 1), (4, 72, 2), (6, 69, 2), (8, 72, 4), (12, 76, 4)],
    [(0, 77, 3), (3, 76, 1), (4, 74, 2), (6, 72, 2), (8, 69, 6), (14, 72, 2)],
    [(0, 79, 3), (3, 76, 1), (4, 72, 4), (8, 76, 2), (10, 79, 2), (12, 84, 4)],
    [(0, 83, 4), (4, 81, 2), (6, 79, 2), (8, 74, 4), (12, 71, 2), (14, 74, 2)],
]
ANSWER = PHRASE[:3] + [[(0, 81, 8), (8, 76, 2), (10, 79, 2), (12, 81, 4)]]


def hz(midi):
    return 440.0 * 2 ** ((midi - 69) / 12)


def pulse(phase, duty):
    return 1.0 if (phase % 1.0) < duty else -1.0


def tri(phase):
    p = phase % 1.0
    return 4 * p - 1 if p < 0.5 else 3 - 4 * p


def render():
    out = [0.0] * TOTAL
    step_len = STEP * RATE

    # Arpeggio: every tick the next chord tone, two octaves, 25 % pulse.
    phase = 0.0
    for i in range(TOTAL):
        t = i / RATE
        bar = int(i / (16 * step_len))
        chord = CHORDS[bar % 4]
        notes = [chord[0] + 12, chord[1] + 12, chord[2] + 12, chord[0] + 24]
        n = notes[int(t / TICK) % len(notes)]
        phase += hz(n) / RATE
        in_step = (i % step_len) / step_len
        env = 0.55 + 0.45 * (1 - in_step)              # a little pump each 16th
        out[i] += 0.10 * env * pulse(phase, 0.25)

    # Bass: 8ths bouncing root / octave, 50 % pulse, gated.
    phase = 0.0
    for i in range(TOTAL):
        s = i / step_len
        bar = int(s / 16)
        root = CHORDS[bar % 4][0] - 12
        eighth = int(s / 2)
        n = root + (12 if eighth % 2 else 0)
        phase += hz(n) / RATE
        gate = 1.0 if (s / 2) % 1 < 0.8 else 0.0
        out[i] += 0.13 * gate * pulse(phase, 0.5)

    # Lead from bar 5: triangle with delayed vibrato, plus an echo.
    lead = [0.0] * TOTAL
    for bar in range(4, BARS):
        part = PHRASE if bar < 12 else ANSWER
        for start, midi, length in part[bar % 4]:
            a = int((bar * 16 + start) * step_len)
            b = min(TOTAL, int((bar * 16 + start + length) * step_len))
            phase = 0.0
            for i in range(a, b):
                k = (i - a) / RATE
                vib = 1 + (0.006 * math.sin(2 * math.pi * 6 * k) if k > 0.12 else 0)
                phase += hz(midi) * vib / RATE
                env = min(1.0, k / 0.01) * (1 - 0.3 * (i - a) / (b - a))
                lead[i] += 0.16 * env * (0.7 * tri(phase) + 0.3 * pulse(phase, 0.125))
    delay = int(3 * step_len)
    for i in range(TOTAL):
        out[i] += lead[i] + 0.35 * lead[i - delay]       # wraps: the loop stays seamless

    # Drums.
    rnd = random.Random(1998)
    for st in range(BARS * 16):
        a = int(st * step_len)
        pos = st % 16
        if pos in (0, 8, 10) or (st // 16 % 4 == 3 and pos == 14):
            for j in range(int(0.18 * RATE)):            # kick: falling sine
                k = j / RATE
                f = 50 + 120 * math.exp(-k * 30)
                out[(a + j) % TOTAL] += 0.45 * math.exp(-k * 14) * math.sin(2 * math.pi * f * k)
        if pos in (4, 12):
            for j in range(int(0.15 * RATE)):            # snare: noise + tone
                k = j / RATE
                n = rnd.uniform(-1, 1)
                out[(a + j) % TOTAL] += math.exp(-k * 22) * (0.25 * n + 0.1 * math.sin(2 * math.pi * 190 * k))
        if pos % 2 == 0:
            prev = 0.0
            for j in range(int(0.04 * RATE)):            # hat: high-passed noise
                n = rnd.uniform(-1, 1)
                out[(a + j) % TOTAL] += 0.07 * math.exp(-j / RATE * 90) * (n - prev)
                prev = n

    # Soft clip, gentle low-pass.
    lp = 0.0
    pcm = bytearray()
    for v in out:
        lp += 0.6 * (math.tanh(v * 1.4) - lp)
        pcm += struct.pack("<h", int(lp * 30000))
    return bytes(pcm)


if __name__ == "__main__":
    with wave.open(sys.argv[1], "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes(render())
