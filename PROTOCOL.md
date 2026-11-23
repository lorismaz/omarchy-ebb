# Amazfit Helio Strap protocol notes

Facts about how an Amazfit Helio Strap (Zepp OS) talks over Bluetooth LE and
how a Zepp account hands out its key, as observed on the wire. These notes are
the only reference Ebb's `ebb-strap` helper is written from. All integers are
little-endian unless stated.

## 1. Standard GATT (no auth)

| What | Service | Characteristic |
|---|---|---|
| Heart rate measurement (notify) | `180d` | `2a37` |
| Battery level (read, notify) | `180f` | `2a19` |
| Software revision (read, UTF-8) | `180a` | `2a28` |

Heart rate measurement (Bluetooth Heart Rate Service spec): byte 0 is flags.
Bit 0 set: bpm is a u16 at 1..3, else a u8 at 1. Bit 2 set: contact detection
is supported, and bit 1 then says whether there is contact. Bit 3 set: a u16
energy field follows and is skipped. Bit 4 set: the rest is u16 RR intervals
in 1/1024 s.

## 2. Zepp channel

Vendor characteristics (base `0000xxxx-0000-3512-2118-0009af100700`):

| Short | Role |
|---|---|
| `0016` | write: frames to the strap; also notifies acks |
| `0017` | notify: frames from the strap |
| `0005` | notify: bulk history data |

### 2.1 Frames

A message `(endpoint: u16, payload)` is sent as one or more frames, each at
most `MTU - 3` bytes (at least 20, at most 512):

```
first frame:  03 | flags | 00 | handle | n | total_len:u32 | endpoint:u16 | data…
later frames: 03 | flags | 00 | handle | n | data…
```

- `handle` (u8) increases by one per message sent; `n` counts frames from 0.
- flags: `01` first frame, `02` last frame, `04` ack wanted (set on the last
  frame), `08` payload encrypted.
- `total_len` is the length of the plain payload.
- When a received frame asks for an ack, answer on `0017` with
  `04 00 handle 01 n`.

Encrypted messages: plain payload, then the sender's sequence number (u32),
then the CRC-32 (zlib polynomial) of those two together, zero-padded to a
multiple of 16 and encrypted with AES-128 in ECB mode. The key is the session
key with every byte XORed with the message handle. The sender's sequence
number increases by one per encrypted message. The receiver decrypts the same
way and keeps the first `total_len` bytes.

### 2.2 Handshake (endpoint `0082`)

Curve: NIST B-163 (sect163r2), `y² + xy = x³ + x² + b` over GF(2¹⁶³) with
reduction polynomial `x¹⁶³ + x⁷ + x⁶ + x³ + 1`; the parameters are the
published NIST ones. Points travel as x then y, each 24 bytes little-endian
(48 bytes per point). Private scalars are random, below 2¹⁶², at least 2⁸¹.

1. Phone → strap: `04 02 00 02` + phone public point (48).
2. Strap → phone: `10 04 01` + strap random (16) + strap public point (48).
3. Shared point = phone scalar × strap point, encoded as above (48 bytes, S).
   - session key = `S[8..24]` XOR the 16-byte auth key
   - phone's first sequence number = u32 at `S[0..4]`
4. Phone → strap: `05` + AES(auth key, strap random) + AES(session key, strap random).
5. Strap → phone: `10 05 01` accepted; `10 05` with anything else: key refused.

### 2.3 Service list (endpoint `0000`)

Request `03`. Reply `04 | count:u16 | count × (endpoint:u16, encrypted:u8)`.
Endpoints used here: `0016` steps, `004b` history fetch. Each is sent
encrypted if its flag says so.

### 2.4 Steps (endpoint `0016`)

- `03`: ask for today's totals. Reply `04 01` + block.
- `05 01`: turn on live updates. Updates arrive as `07` + block.
- Block (13 bytes): `flags:u8, steps:u32, distance_m:u32, kcal:u32`. Only
  steps is reliable; treat distance ≥ 1 000 000 or kcal ≥ 100 000 as missing.

### 2.5 History fetch (endpoint `004b`, data on `0005`)

- Start: `01 type` + time8(since). Types: `48` sleep sessions, `01` per-minute
  activity.
- time8: `year:u16, month, day, hour, minute, second, tz` where tz is the UTC
  offset in 15-minute steps (signed).
- Start reply: `10 01 01 count:u32` + time8 of the first record (when present).
  Status other than `01` in byte 2 is an error. Count 0 means nothing to send.
- `02`: send the data. Data arrives on `0005` as `counter:u8` + bytes; the
  counter runs 0, 1, 2… (wrapping at 256); a gap means the transfer is broken.
- Done reply: `10 02 01` + optional `crc32:u32` of all data bytes.
- Acknowledge with `03 09`: done, but keep the data on the strap so the phone
  app can still sync it. (`03 01` would mark it as synced and the strap would
  drop it.) Ack reply: `10 03 01`.

### 2.6 Sleep session record (type `48`), 594 bytes each

| Offset | Size | Meaning |
|---|---|---|
| `0x00` | u32 | when the record was written (epoch s) |
| `0x04` | u32 | local midnight ending the night (epoch s) |
| `0x15` | u8 | average heart rate |
| `0x16` | u8 | sleep score |
| `0x54` | u8 | number of stages (at most 100 fit) |
| `0x56` | 5 × n | stages: `start:u16, end:u16, kind:u8`, minutes from the previous midnight (`midnight − 24 h`); `end` is the last minute included |
| `0x24A` | 4 × u16 | totals in minutes: REM, light, deep, awake |

Stage kinds: 4 light, 5 deep, 7 awake, 8 REM. A later-written record for an
overlapping night replaces the earlier one.

### 2.7 Per-minute activity (type `01`), 8 bytes a minute

From the start time in the start reply: `kind, intensity, steps, heart rate,
?, sleep, deep, rem`. Kinds seen: 115 not worn, 118 charging, 120 asleep.

## 3. Zepp account (key download)

Zepp's private API, used the way the Zepp app uses it. Email and password
accounts only.

1. `POST https://api-user-us2.zepp.com/v2/registrations/tokens`, header
   `x-hm-ekv: 1` plus the app headers. The body is the form
   `emailOrPhone, password, state=REDIRECTION, client_id=HuaMi,
   redirect_uri=https://s3-us-west-2.amazonaws.com/hm-registration/successsignin.html,
   region=us-west-2, token=access, token=refresh, country_code=US`,
   encrypted with AES-128-CBC and PKCS#7 padding, key `xeNtBVqzDc6tuNTh`,
   IV `MAAAYAAAAAAAAABg` (the app's fixed values). Success is a `303` whose
   `Location` query holds `access` and `refresh`; otherwise it holds `error`.
   Do not follow the redirect.
2. `POST https://api-mifit-us2.zepp.com/v2/client/login` with the form
   `code=<access>, grant_type=access_token, third_name=huami, app_name=com.huami.midong,
   country_code=US, device_id=<random uuid>, device_model=android_phone, …`.
   The reply JSON has `token_info.login_token`, `.app_token` and `.user_id`.
3. `GET https://api-mifit.zepp.com/users/<user_id>/devices?…` with header
   `apptoken: <app_token>`. The reply's `items[]` each have `macAddress`,
   `activeStatus`, and `additionalInfo`, a JSON string whose `auth_key` is the
   32-hex-digit key.
4. `POST https://api-mifit-us2.zepp.com/v1/client/logout` with
   `login_token=<login_token>`.

App headers: `appname`/`app_name: com.huami.midong`, `appplatform: android_phone`,
`v: 2.0`, `cv: 151689_9.12.5`, `vn: 9.12.5`, `vb: 202509151347`,
`user-agent: Zepp/9.12.5 (Pixel 4; Android 12; Density/2.75)`.

## 4. Key storage

The key is kept in the desktop keyring (Secret Service) under the attributes
`service=omarchy-ebb` (before Ebb 1.3.0, `omarchy-helio-strap`, still read), `address=<MAC in upper case>`, as 32 lower-case
hex digits.
