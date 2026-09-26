# Linux voice companion: implementer's spec

> **Status:** the implementer's spec written before voice was built (2026-09-24/25), kept because
> the code cites its sections. Where the shipped app differs, the code is right. The main
> difference: the companion floats as the mascot alone, with no card or frame, and its glow takes the
> status colour as the macOS pet's does (§2.4 suggested a card). Paths such as `~/src/...` are the
> author's checkouts.

Target: the Rust GTK4/libadwaita app in `~/src/fermix-linux-desktop/desktop` (`io.tezra.Fermix`,
GNOME 50 Flatpak runtime, gtk4-rs 0.11.5 / glib 0.22.10 / libadwaita 0.9.2 per `Cargo.lock`),
talking to the host `fermix` package (0.11.0 installed, `fermix.service` user unit).
Researched 2026-09-24/25, read-only. No voice session was started and nothing under `~/.fermix`
was written.

Repos cited:
- **E** = `~/src/fermix/apps/fermix_core` (engine)
- **M** = `~/src/fermix-macos/Apps/Fermix/Sources/FermixAppCore` (macOS app)
- **L** = `~/src/fermix-linux-desktop/desktop` (Linux app)
- **M38** = `~/src/fermix/docs/design/MILESTONE_38_LINUX_COMPANION_APP (1).md`

---

## 0. Decisions in one screen

| Question | Answer |
|---|---|
| Wire | `$FERMIX_HOME/realtime.sock` (default `~/.fermix/realtime.sock`), NDJSON, hello-first, client speaks **protocol 2** (daemon window 1..2) |
| Audio on the wire | **PCM16 little-endian, mono, 24 000 Hz**, base64 in JSON. Uplink chunk ≤ 16 384 decoded bytes (≈341 ms); send **100 ms = 4 800 bytes** |
| Turn taking | Full duplex, open mic, **server VAD only**. No push-to-talk exists on the wire. Controls: begin/end call, mute, interrupt (stop reply), cancel task (Live only) |
| Audio stack | **GStreamer via gstreamer-rs 0.25** (all of GStreamer 1.26.11 is in the GNOME 50 runtime): `pulsesrc → webrtcdsp (AEC) → 24 kHz → appsink` and `appsrc → 48 kHz → webrtcechoprobe → pulsesink` |
| Flatpak | Add exactly one finish-arg: `--socket=pulseaudio`. Keep `--filesystem=~/.fermix:ro` (the realtime socket sits beside `daemon.sock`, which already works through it) |
| Portal | None. There is no microphone or audio portal; the Camera portal is irrelevant. No in-app "Allow microphone?" dialog |
| Presence | A **Voice page** in the main window, plus an optional small **companion window**. On GNOME the app cannot pin itself on top; the user can, through the window menu (Alt+Space → Always on Top). Say so |
| Pet art | Reuse the 15 macOS PNG layers as is (first-party, MIT). Downscale to 256 px at build time |
| Prerequisites | Voice enabled in config, an OpenAI **Platform API key** (a ChatGPT/Codex sign-in does not count), then a daemon restart, because voice is boot-bound |

---

## 1. The realtime wire

### 1.1 Transport

- **Socket:** Unix stream socket at `$FERMIX_HOME/realtime.sock`, default `~/.fermix/realtime.sock`
  (E `lib/fermix_core/realtime/config.ex:235-244`; E `priv/realtime/PROTOCOL.md:14-15`). Mode `0600`,
  created by the daemon (E `realtime/local_voice_socket.ex:82-88`). The Linux app already resolves
  `$FERMIX_HOME` or `~/.fermix` (L `app/src/daemon.rs:44-49`). Reuse that function.
- **When it exists:** only when the daemon **booted** with `realtime.enabled = true`, provider `openai`
  and an OpenAI API key present (E `lib/fermix_core/application.ex:396-399`, `:477-482`). Otherwise
  there is no socket (ENOENT), even while the daemon is otherwise healthy. On the owner's host today
  `[fermix_core.realtime] enabled = false`, so there is no `realtime.sock`. `daemon.sock`, `acp.sock`
  and `browser_bridge.sock` are present.
- **Framing:** one JSON object per line, terminated by `\n`, with no length prefix. This differs from
  `daemon.sock`, which uses a 4-byte length prefix (L `core/src/frame.rs`).
- **Daemon's inbound line cap:** 65 536 bytes, including the base64 audio. If a line exceeds it, the
  daemon replies `error: line_too_large` and closes the connection (E `local_voice_socket.ex:27`,
  `:440-441`).
- **Client's inbound caps:** use the macOS values. Frames are limited to 1 MiB and the unscanned buffer
  to 2 MiB (M `Voice/RealtimeProtocol.swift:19-23`). The contract publishes no ceiling for daemon→client
  frames. `audio_delta` size is whatever the provider sent.
- **Client cap:** at most 4 concurrent connections. The fifth gets `{"type":"error","reason":"max_clients_reached"}`
  and is closed (E `local_voice_socket.ex:28`, `:260-269`).
- **Peer EOF means teardown.** When the client closes the socket or crashes, the daemon stops the
  session (E `local_voice_socket.ex:374-375`, `:413-416`). That is what keeps a billed provider
  socket from outliving the app. The client should still send `call_stop` when it can.
- **Flatpak reachability:** the manifest mounts `~/.fermix` read-only, and connecting to a socket
  needs no write access. This is measured for `daemon.sock` (L `README.md` "Sandbox"). `realtime.sock`
  sits in the same directory, so it needs no new filesystem permission.

### 1.2 Versioning and handshake

- Daemon constants: `@protocol_version 2` and supported window `{1, 2}` (E `realtime/protocol.ex:34-35`,
  `:47-48`). The installed 0.11.0 ships byte-identical `priv/realtime/` exports: `PROTOCOL.md`, the
  schema and both fixture files match HEAD (compared against
  `~/.local/share/.burrito/fermix_linux_package_erts-16.3_0.11.0/lib/fermix_core-0.11.0/priv/realtime`).
  v0.10.5 was still protocol 1, so a v2 client needs **fermix ≥ 0.11.0**.
- The client declares **2**. A v1 client can still run the Realtime engine, but it is refused at
  `call_start` under Live (E `local_voice_socket.ex:480-486`, `:621-642`). There is no reason to ship
  v1.

The sequence:
```
client: connect(realtime.sock)
client → {"type":"client_hello","protocol_version":2}
daemon → {"type":"server_hello","min_version":1,"max_version":2}      (if 2 ∈ [min,max])
       | {"type":"error","reason":"unsupported_protocol_version","direction":"client_too_old"|"client_too_new",
          "client_version":2,"min_version":N,"max_version":M}  then close
client: validate 2 ∈ [min_version, max_version] itself → negotiated
```

Rules:
- **Daemon:** any event before `client_hello` returns `error: handshake_required` and closes. A
  second `client_hello` returns `error: unexpected_client_hello` and closes (E `local_voice_socket.ex:472-474`,
  `:646-648`).
- **Client:** apply a **3 s** handshake deadline (M `RealtimeProtocol.swift:26`). While waiting,
  ignore every frame except `server_hello` and `error` (M `Voice/VoiceSession.swift:380-391`). Treat a
  window that excludes 2 as "update required", with a direction. A second `server_hello` is ignored
  (M `VoiceSession.swift:411-416`).
- **Vendoring:** vendor the contract the way macOS does. Copy
  `priv/realtime/{PROTOCOL.md,protocol.schema.json,fixtures/*.jsonl}` byte for byte into L (for
  example `core/contracts/realtime/`) with a `CHECKSUMS.txt`. Today's SHA-256 values, identical to
  macOS's pin (M `Resources/Contracts/CHECKSUMS.txt`):
  `PROTOCOL.md 63b57677…600`, `protocol.schema.json fcc0c865…ef79`,
  `client_events.jsonl b53a25c0…d33d`, `server_events.jsonl 81a1cd98…7321`.
  A core test decodes every golden server line and re-encodes every golden client line.

### 1.3 Client → daemon events

The catalogue is E `PROTOCOL.md:124-134`. Validation is E `realtime/protocol.ex:113-169`.

| type | fields | when | notes |
|---|---|---|---|
| `client_hello` | `protocol_version` int > 0 | first frame | see 1.2 |
| `call_start` | none | user begins a call | starts the provider session (billing starts). Needs a completed handshake |
| `audio_chunk` | `audio`: base64 PCM16 LE mono 24 kHz, non-empty | continuously while `listening`/`speaking`/`thinking` and not muted | decoded size must be ≤ `max_chunk_bytes` (default **16 384**; E `config.ex:84`, `:204`), else `error: {:chunk_too_large, n, 16384}` and close. Keep chunks an even byte count (Live refuses odd frames; E `realtime/openai_live_client.ex` `audio_append_event`). Sending one before `call_start` returns `error: not_connected` and closes (E `local_voice_socket.ex:498-510`, `:721-724`) |
| `interrupt` | `audio_end_ms` int ≥ 0, optional | user presses Stop | Realtime: truncates the current assistant item at `audio_end_ms`, cancels the response, echoes `playback_stop` and `state:listening` (E `realtime/session_server.ex:252-265`). Live: ignores `audio_end_ms` and injects "Stop speaking now and listen." (E `realtime/live_session_server.ex:168-185`) |
| `mute` | `enabled` bool (default true) | mute toggle | daemon drops chunks while muted and reports `state:muted`/`listening` (E `session_server.ex:270-273`, `:306-309`; Live E `live_session_server.ex:187-196`). The client **also** stops sending locally (M `Voice/AudioOwner.swift:143-146`) |
| `call_stop` | none | user ends the call | ends the call and keeps the connection. Daemon answers `state:idle` (E `local_voice_socket.ex:544-551`). A new `call_start` may follow on the same connection |
| `task_cancel` | `delegation_id` non-empty | Live only, running task | under Realtime it returns `error: unsupported_by_engine` and closes (E `local_voice_socket.ex:488-496`). Send only when `call_ready.engine == "openai_live"` |

Examples (golden, E `priv/realtime/fixtures/client_events.jsonl`):
```json
{"type":"client_hello","protocol_version":2}
{"type":"call_start"}
{"type":"audio_chunk","audio":"<base64 of 4800 bytes>"}
{"type":"interrupt","audio_end_ms":1500}
{"type":"mute","enabled":true}
{"type":"task_cancel","delegation_id":"dg_01H9"}
{"type":"call_stop"}
```

### 1.4 Daemon → client events

The catalogue is E `PROTOCOL.md:136-151`. **The wire carries fields the schema does not list.**
Decode leniently: ignore unknown fields, and log unknown `type`s without closing (PROTOCOL rule 4,
E `PROTOCOL.md:93`).

| type | fields | meaning / client action |
|---|---|---|
| `server_hello` | `min_version`, `max_version` | handshake reply |
| `state` | `state`: `idle` `listening` `speaking` `muted` `thinking` `reconnecting` (**open vocabulary**) | turn state. Unknown values render as idle (E `PROTOCOL.md:141`). `listening` is the signal to **start streaming mic audio** (M `App/AppModel.swift:354-387`) |
| `audio_delta` | `audio`: base64 PCM16 LE mono 24 kHz | enqueue for playback. The pet shows speaking |
| `playback_stop` | none | **flush the playback queue now** and reset the utterance anchor |
| `transcript_delta` | `text`; Realtime also sends `role:"user"` | Realtime only. It carries the **whole** user utterance once it is transcribed, not a delta (E `session_server.ex:814-817`) |
| `assistant_text_delta` | `text` | Realtime only. Streamed deltas, **then the full final text again** (E `session_server.ex:809-812` and `:876-879`). See risk R6. macOS ignores both transcript events (M `AppModel.swift:346`) |
| `tool_event` | `status` (`running`/`completed`/`error`), `name`, `reason?` | Realtime tools. **The daemon sends `running`** (E `session_server.ex:882`). macOS only maps `started`/`completed`/`error`, so `running` falls into `.unrecognized` (M `RealtimeProtocol.swift:171-178`). Map `running`→tool use, `completed`→back to listening, `error`→show `reason` |
| `usage` | engine-specific, see below | cost facts. Never a state change |
| `error` | `reason`, plus `kind?`, `detail?`, `direction?`, `client_version?`, `min_version?`, `max_version?`, `required_for?` | **terminal for the call**. The daemon closes after most errors. The client must tear capture down (M `AppModel.swift:454-463`) |
| `call_ready` | `engine`, `call_id`, `provider_session_id?`, `expires_at?`, `captions` | **Live only** (v2). The Realtime engine never sends it |
| `caption` | `speaker` `user`/`assistant`, `delta`, `start_ms`, `end_ms` | Live only. Concatenate bytes verbatim. Speakers may overlap |
| `task` | `delegation_id`, `revision`, `status` `pending`/`running`/`completed`/`failed`/`cancelled`, `summary?` (≤240 chars) | Live only. `revision` fences a re-asked task |

`usage` shapes seen on the wire:
```json
{"type":"usage","status":"estimated","estimated":{"input_audio_ms":1200,"input_audio_tokens":0,"cost_cents":0.12,"transcription_ms":1200,"transcription_cost_cents":0.01},"reported":{"cost_cents":0.0}}
{"type":"usage","status":"reported","cost_cents":1.84}
{"type":"usage","status":"limit_reached","reason":"cost_limit"}
{"type":"usage","status":"live","voice_seconds":64.2,"voice_cost_cents":5.35,"backend_turns":2,"backend_cost":"unknown","accounting":"running"}
```
Rows 1–3 are Realtime (E `session_server.ex:1549-1556`, `:1401-1413`, `:371-376`). Row 4 is Live
(E `realtime/live_ledger.ex:161-170`). `backend_cost:"unknown"` is never rendered as 0.

`error` examples:
```json
{"type":"error","reason":"unsupported_protocol_version","direction":"client_too_new","client_version":99,"min_version":1,"max_version":2}
{"type":"error","reason":"unsupported_protocol_version","kind":"update_required","direction":"client_too_old","client_version":1,"min_version":2,"max_version":2,"required_for":"openai_live"}
{"type":"error","reason":"max_session_duration"}
{"type":"error","reason":"cost_limit","kind":"cost_limit"}
{"type":"error","reason":"provider_send_failed: …"}
```
The second example is a Live refusal. The third and the first half of the fourth are Realtime, which
sends **no `kind`**; the full fourth example is Live.

Error reasons a client can meet, all from E. Realtime engine errors carry **no `kind`**, so map by
`reason` as well as by `kind`:

| reason | source | user meaning |
|---|---|---|
| `handshake_required`, `unexpected_client_hello`, `invalid_json`, `invalid_event`, `missing_type`, `{:unknown_event, "x"}`, `missing_audio`, `invalid_audio_base64`, `{:chunk_too_large, n, max}`, `invalid_audio_end_ms`, `missing_delegation_id`, `missing_protocol_version`, `invalid_protocol_version`, `line_too_large` | protocol violations (E `protocol.ex:94-169`, `local_voice_socket.ex:440-474`) | client bug. Log it, show "Voice stopped: the app and Fermix disagreed about the call." |
| `unsupported_protocol_version` (+`direction`) | handshake or Live `call_start` | "Update the app" (`client_too_old`) or "Update Fermix" (`client_too_new`) |
| `max_clients_reached` | 5th connection | "Four other voice clients are connected to Fermix." |
| `not_connected` | audio/interrupt/mute before `call_start` | client bug |
| `unsupported_by_engine` | `task_cancel` under Realtime | client bug |
| `not_configured` | `call_start` with no OpenAI key (E `local_voice_socket.ex:61-77`; E `lib/fermix_core/config.ex:33-40`) | prerequisite (section 4) |
| `provider_send_failed: …` | provider socket could not open or configure (E `session_server.ex:1315-1320`) | "OpenAI refused the voice call." Show the text; often a bad or unauthorised key |
| `provider_disconnected` | reconnect exhausted after 1 s, 2 s and 4 s (Realtime; E `session_server.ex:53`, `:418-452`) or immediately (Live; no reconnect, E `live_session_server.ex:14-18`) | "The connection to OpenAI dropped." |
| `max_session_duration` | `realtime.max_session_minutes` (default 15; E `config.ex:86`) | "Calls end after N minutes." |
| `cost_limit` | per-call ceiling (default 100 cents; E `config.ex:87`) | "This call reached its spending limit." |
| `session_expired`, `close_timeout`, `bridge_unavailable`, `provider_refused` (+`detail`) | Live (E `realtime/live_frames.ex:118-127`) | show `detail` verbatim when present |
| `{:session_down, reason}` | session process died (E `local_voice_socket.ex:421-426`) | "Voice stopped unexpectedly." |

Non-terminal provider hiccups are deliberately **not** sent as `error`, because the client would
treat them as terminal (E `session_server.ex:432-441`, `:908-919`). A mid-call provider drop shows as
`state:reconnecting`. macOS renders `reconnecting` as idle (M `Voice/VoicePresentation.swift:211`).
Linux should say "Reconnecting…" instead: the value is documented and costs nothing to show.

### 1.5 Audio format and timing

- **Uplink:** PCM16 LE, mono, **24 000 Hz**. The daemon converts at `@pcm16_bytes_per_ms 48`
  (E `session_server.ex:28`) and declares `audio/pcm` rate 24 000 to the provider
  (E `realtime/openai_client.ex:406-412`; Live E `openai_live_client.ex:38`, `:117`). Config refuses
  any format other than `pcm16` (E `config.ex:292-293`).
- **Downlink:** the same format (E `openai_client.ex:410-412`).
- **Chunking:** macOS sends one chunk per ≈100 ms capture buffer (4 800 frames at the device rate,
  converted to 24 kHz; M `Voice/AudioController.swift:8-9`, `:156-174`). Linux: fixed **2 400 samples = 4 800
  bytes = 100 ms**, which encodes to 6 400 base64 characters, well under both the 16 384-byte decoded
  cap and the 65 536-byte line cap.
- **Keep sending silence.** Stream continuously during a call, including silence. The provider's
  VAD needs the quiet to find where speech starts (E `session_server.ex:340-346`), and Live expects
  "continuous PCM, including silence" (E `PROTOCOL.md:164`). The only exception is the muted state.
- **Turn detection** is fixed server-side: `server_vad`, threshold 0.6, prefix 300 ms, silence
  800 ms, `create_response` and `interrupt_response` both true, `noise_reduction: near_field`
  (E `openai_client.ex:135-140`, `:414-426`). Config keys `turn_detection`/`activation` are
  rejected: "realtime uses one full-duplex server_vad mode" (E `config.ex:173-177`, `:325-329`).
  **Push-to-talk is not a wire feature.** A client-side hold-to-talk (unmute while held) would still
  leave turn ends to the server's VAD. Out of scope for v1.

### 1.6 Session lifecycle

```
Disconnected --connect()--> Connecting --ok--> Handshaking --server_hello ok--> Negotiated(idle)
Negotiated --user Begin--> [warm capture muted] --> send call_start --> CallStarting
CallStarting --(Live) call_ready--> CallStarting
CallStarting/InCall --state:listening--> InCall: start streaming mic (idempotent)
InCall --state:speaking/thinking/muted/reconnecting--> InCall (presentation only)
InCall --audio_delta--> play;  --playback_stop--> flush playback
InCall --user End--> send call_stop --> state:idle --> Negotiated (connection kept)
any --error frame | EOF | write failure | handshake timeout--> tear down capture + playback → Disconnected(reason)
```

- Connect lazily when the user first presses Begin, as macOS does (M `Voice/VoiceCoordinator.swift:84-96`,
  `:134-141`). Keep the connection after `call_stop`. There is no reconnect loop: the next Begin
  reconnects.
- Warm capture **before** `call_start` but keep it muted and detached from the socket. Start sending
  only on `state:listening` (M `AudioOwner.swift:95-141`; M `AppModel.swift:372-374`). Never send
  audio before the daemon says it is listening.
- Realtime: `call_start` sends `session.update`. `state:listening` follows `session.updated`
  (E `session_server.ex:213-235`, `:830-837`). Live: `call_ready` then `state:listening`
  (E `live_session_server.ex:386-400`). The daemon's own `call_start` timeout is 10 s
  (E `session_server.ex:51`). Use a **12 s** client deadline from `call_start` to first `listening`
  before showing "Fermix did not start the call."
- Realtime state sources:
  - `speaking` comes from each audio delta (E `session_server.ex:799-801`).
  - `thinking` comes from speech stopping with no reply in flight (`:868-874`).
  - `playback_stop` comes from a committed user turn, which is server barge-in (`:851-854`), or a
    cancelled response (`:1558-1563`).
- Live: `speaking` on the first delta, `listening` again when a user caption arrives
  (E `live_session_server.ex:410-423`, `:857-858`).
- On app shutdown or last-window close, send `call_stop` if a call is active, close capture, then
  close the socket. The daemon also treats EOF as teardown (M `VoiceCoordinator.swift:68-80`).

### 1.7 Interruption

1. **Server barge-in**, the normal path. The user talks over the reply. The provider's VAD cancels or
   commits and the daemon sends `playback_stop`. The client flushes. Nothing is sent by the client.
2. **Explicit Stop button.** Compute `played_ms`, the audio of the current utterance that actually
   reached the speaker. Flush playback locally first, then send `{"type":"interrupt","audio_end_ms":played_ms}`
   (M `VoiceCoordinator.swift:56-60`, `AudioOwner.swift:176-180`). The utterance anchor is set on
   the first `audio_delta` after a reset. Reset it on `playback_stop` and whenever state leaves
   `speaking` (M `AppModel.swift:376-378`, `:389-398`). Show Stop only while thinking or visually
   speaking (M `Pet/PetFeatureModel.swift:145-147`).

### 1.8 Client transport constants

Port these from M `Voice/RealtimeSocketClient.swift`:

| constant | value | cite |
|---|---|---|
| outbound audio queue | 20 chunks (≈2 s), **drop oldest** | `:89`, `:340-363` |
| control frames | never dropped. If one is not flushed in **5 s**, the connection is dead | `:94`, `:446-468` |
| audio stall | queue saturated with no write progress for **8 s** means the connection is dead | `:105`, `:472-493` |
| SIGPIPE | disable. Rust `UnixStream` ignores SIGPIPE by default through std's runtime, so writes return `EPIPE` | `:260-273` |

---

## 2. macOS behaviour to match

### 2.1 States and how the pet shows them

Voice modes (M `Voice/VoicePresentation.swift:192-215`) and the pet's expression
(M `Pet/PetExpression.swift:9-24`):

| mode | from | expression | tint (palette) | status copy (M `Resources/en.lproj/Localizable.strings:322-333`) |
|---|---|---|---|---|
| offline | no socket / failure | idle | faint | "Not connected" |
| idle | negotiated, no call (also `reconnecting` and unknown states on macOS) | idle (listening if a call is active) | secondary | "Ready" |
| listening | `state:listening` | listening | accent | "Listening" |
| muted | `state:muted` or local mute | listening | warning | "Muted" |
| thinking | `state:thinking` | thinking | secondary | "Thinking" |
| speaking | `audio_delta`/`state:speaking` **or audio still playing out** | speaking | success | "Speaking" |
| toolUse | `tool_event` / non-terminal `task` | thinking | secondary | "Running a tool" |
| error | `error` frame / capture failure | idle | error | the daemon's words: "The daemon reported: %@" |

Rules to port exactly:
- **Visual speaking outlasts the wire state.** While playback is still draining, the pet reads as
  speaking even after the daemon says `listening` (M `VoicePresentation.swift:305-310`). The playback
  queue's "drained" event clears it (M `AppModel.swift:309-314`).
- **The daemon is authoritative about mute.** `state:muted` sets local mute, and `state:idle` clears it
  (M `AppModel.swift:360-366`).
- `call_began` clears the previous call's captions, task, usage, engine and call id
  (M `AppModel.swift:227-240`).
- Captions keep the last 40 fragments (M `AppModel.swift:30-32`, `:424-436`). The surface shows the
  last fragment as "You: …" or "Fermix: …", on one line, truncated.
- A task is terminal only for `completed`/`failed`/`cancelled`. An unknown status is not terminal
  (M `Voice/RealtimeProtocol.swift:269-274`). Cancel is offered only while `running`
  (M `PetFeatureModel.swift:135`).
- Nothing is conveyed by colour alone. Every state has a word and an icon
  (M `VoicePresentation.swift:288-336`).

### 2.2 Mascot rendering, to port as pure functions in `core`

- **Layers per expression:** `ring` (drawn at **1.20×**, behind), `body`, `face`, and `decor`
  (at 0.75 opacity, only where present), then `pet_ball` on top at **y −15 pt**, shared across
  expressions (M `Pet/PetView.swift:230-245`, `:188-199`).
- **Expression change:** crossfade with a 0.97 scale pop, and ease motion in over **0.5 s**
  (smoothstep) (M `PetView.swift:116-157`, `:188-200`).
- **Motion per mode:** breath (scale), bob (y) and sway (rotation, 4.2 s period) are sine waves with
  independent periods. The tables are M `Pet/MascotMotion.swift:136-183`. Speaking adds
  `+0.06 × output RMS` to the scale (M `PetView.swift:159-164`). RMS is computed per downlink chunk
  and smoothed with α = 0.35 (M `AudioController.swift:296-317`, `AudioOwner.swift:57`, `:78-84`).
- **Blink:** the idle (closed-eye) face crossfades over the listening and thinking faces on a jittered
  ≈2.8 s cycle (M `MascotMotion.swift:192-208`, `PetView.swift:252-262`).
- **Speaking-face registration fix:** offset `(−12, +24)` px on the 1024 px canvas, scaled to the
  display (M `PetView.swift:274-282`).
- **Sizes:** window 180×168, stage 132×116, mascot 116×108 pt (M `PetFeatureModel.swift:6-13`).
  30 fps timeline, **paused when the window is not visible or reduce-motion is on**
  (M `PetView.swift:95`, `:110-114`).
- **Idle glow:** tint at 0.18 opacity, or 0.34 during a call (M `PetView.swift:74-78`).

GTK4 mapping:
- Write one custom `gtk::Widget` subclass whose `snapshot()` draws the layer `gdk::Texture`s with
  `snapshot.translate/scale/rotate/push_opacity`.
- Drive it with `widget.add_tick_callback`. The frame clock stops ticking when the widget is
  unmapped, which is the macOS "window not visible" pause for free.
- Honour `gtk::Settings::is_gtk_enable_animations()` as reduce-motion: keep the expression, stop the
  motion.
- Keep the motion and crossfade maths in `core` (pure, `f64` time in, transform out) so it tests
  without GTK.

### 2.3 Controls

- **Open mic, full duplex**, as on macOS. There is no push-to-talk and no global hotkey on macOS
  (M `App/CommandTable.swift` has no shortcut for the pet).
- **Begin/End call:** the primary button. On the floating pet, tapping the mascot toggles the call
  (M `PetView.swift:22`). Copy: "Begin voice call"/"End voice call".
- **Mute/Unmute microphone:** only while a call is active.
- **Interrupt reply ("Stop"):** only while thinking or speaking.
- **Cancel task:** Live only, while a task is running.
- **Context menu** on the pet: call, mute (in a call), interrupt, open Fermix (M `PetView.swift:55-65`).
- **Control dock:** appears on hover, during a call, or while speaking (M `PetView.swift:70-72`,
  `:299-338`).
- **Live-call block** on the Pet page: last caption line, task status, and "Voice so far: $x.xx"
  (M `Pet/PetSurfaceView.swift:267-300`).

### 2.4 Window behaviour on macOS and the GNOME-honest equivalent

macOS:
- The floating pet is a borderless, transparent, **floating-level** 180×168 non-resizable window on
  all Spaces, dragged from anywhere (M `App/WindowCoordinator.swift:134-145`,
  `App/AppKitWindowHost.swift:275-284`, `PetView.swift:35-48`).
- It is hidden until the user turns it on, with the hint "A small always-on-top window you can leave
  open beside your work." (M `PetFeatureModel.swift:155-169`; `Localizable.strings:358-360`).
- The Pet sidebar page is the primary surface: preview, status, controls and live-call block
  (M `PetSurfaceView.swift:210-316`).

Linux constraints:
- **Wayland** has no protocol for a client to pin itself above others or to place itself at screen
  coordinates. GTK4 removed `gtk_window_set_keep_above`. Mutter does not implement layer-shell.
  M38 records the same constraint (§8.3, lines 2903-2923).
- **Windows are the presence model.** Closing the last window quits the GUI (M38 §5.2, 1532-1593),
  so a call cannot outlive every window.

Proposal:
1. **Voice page** in the main window. This is the equivalent of the macOS Pet page, and it is the
   primary, always-reachable surface. It shows the mascot, the status line, the controls, the
   live-call block and the prerequisites (section 4).
2. **Companion window** (optional, off by default, toggled from the Voice page as on macOS):
   - A second `gtk::Window` of the same `GtkApplication`: `decorated(false)`, `resizable(false)`,
     about 180×200.
   - Content inside a `gtk::WindowHandle`, which gives compositor-driven drag on Wayland and X11.
   - Draw an `.osd`/card-styled rounded background rather than relying on full transparency, so it
     also looks right without a compositor.
   - Primary click on the mascot toggles the call. Secondary click opens a `gtk::PopoverMenu` with
     Begin/End, Mute, Stop, Open Fermix, and **"Keep on top…"**. That item calls
     `gdk::Toplevel::show_window_menu(event)`, which opens GNOME's own window menu where
     **Always on Top** lives. If it returns `false`, the compositor offers no window menu and the
     item is hidden.
   - Footer copy (new, replacing the macOS hint): "A small window you can keep beside your work.
     On GNOME, Alt+Space then Always on Top keeps it above other windows." (Shortened 2026-09-26 at
     the owner's request for a cleaner Voice page; it still never claims Fermix can pin it.)
   - The window's position is the compositor's choice and is not restored. Do not claim otherwise.
   - A call keeps running while either window is open. Closing the last window ends the call:
     send `call_stop`, then release the microphone.
3. No tray and no X11 keep-above hack. The owner's desktop is X11 today (Pop!_OS 22.04), but a
   separate X11 path would ship behaviour Wayland users cannot get.

---

## 3. Linux audio: recommendation

### 3.1 What the platform gives, measured on this host

- **GNOME 50 runtime** (`org.gnome.Platform//50`):
  - GStreamer **1.26.11**, with `pulsesrc`, `pulsesink`, `pipewiresrc`, `pipewiresink`, `appsrc`,
    `appsink`, `audioconvert`, `audioresample`, `level` and `volume`.
  - **`webrtcdsp` and `webrtcechoprobe`** from gst-plugins-bad, linked against
    **`libwebrtc-audio-processing-2.so.1`** (the modern AEC3 generation).
  - `libpulse` 17, `libpipewire-0.3` 1.6.8, `libasound`, `libspeexdsp` and `libsamplerate`.
- **GNOME 50 SDK:** `pkg-config` finds `gstreamer-{1.0,app-1.0,audio-1.0,base-1.0}` 1.26.11,
  `libpipewire-0.3` 1.6.8, `libpulse` 17.0 and `webrtc-audio-processing-2` 2.1. rust-stable
  extension 25.08 ships **rustc 1.98.1**.
- **Nothing needs to be vendored as a C module.**
- **`webrtcdsp` caps:** S16LE interleaved or F32LE non-interleaved, at **48 000, 32 000, 16 000 or
  8 000 Hz only**. 24 kHz is not accepted, so the DSP runs at 48 kHz and the stream is resampled to
  24 kHz after it. Defaults: echo-cancel on, noise-suppression on (moderate), gain-control on
  (adaptive-digital), high-pass on, limiter on, probe name `webrtcechoprobe0`.
- **Flatpak sandbox:**
  - With `--socket=pulseaudio`, `gst-device-monitor-1.0 Audio/Source` inside the SDK sandbox lists
    the host's three sources (two ALSA inputs and a monitor) as `pulsesrc device=…`.
  - **Without it, no sources are visible.**
  - With `--socket=pulseaudio`, **`pipewire-0` is not present** in the sandbox's `XDG_RUNTIME_DIR`
    (Flatpak 1.14.6). A native PipeWire path would need `--filesystem=xdg-run/pipewire-0`, and
    PipeWire restricts Flatpak clients anyway.
  - Only devices were listed; no stream was opened.
- **Host:** PipeWire 1.0.3 with pipewire-pulse ("PulseAudio (on PipeWire 1.0.3)") and WirePlumber
  0.4.17. **No echo-cancel module is loaded**, although `libspa-aec-webrtc.so` is installed. The
  default source is the laptop's internal mic and the default sink its analog speakers, so the
  speaker-to-mic echo path is real on the owner's machine.
- **Portals:** there is no Microphone or Audio portal. M38 records xdg-desktop-portal 1.22.1
  (2026-06-17) as having Camera and no Microphone or Audio (§7.3, §7.5). Upstream is still at the
  discussion stage ([xdg-desktop-portal#1129](https://github.com/flatpak/xdg-desktop-portal/issues/1129),
  [discussion #1142](https://github.com/flatpak/xdg-desktop-portal/discussions/1142),
  [#615 microphone portal](https://github.com/flatpak/xdg-desktop-portal/issues/615)).
  `--socket=pulseaudio` is the standard route and is documented by Flatpak
  ([sandbox permissions](https://docs.flatpak.org/en/latest/sandbox-permissions.html)). The Camera
  portal has nothing to do with audio. **Do not call any portal for voice.**

### 3.2 Does macOS use voice-processing I/O?

No. `AudioController` never enables it: it only *disables* it at shutdown in case it is on, and logs
its state (M `Voice/AudioController.swift:211-216`, `:238`). Git history shows no enabling commit.
macOS runs raw capture plus server-side `near_field` noise reduction and a raised VAD threshold (0.6)
to resist self-interruption. On Linux laptops, speaker-to-mic coupling through the provider's VAD
triggers **self-barge-in** (the assistant interrupts itself). On-device AEC is the Linux answer, and
it costs nothing extra: `webrtcdsp` is already in the runtime.

### 3.3 Recommendation: GStreamer through gstreamer-rs, one pipeline per call

Crates (app crate only; the `core` crate stays GTK- and GStreamer-free):
```toml
gstreamer       = "0.25"   # 0.25.4, depends on glib ^0.22, the same glib as gtk4 0.11.5
gstreamer-app   = "0.25"   # 0.25.2
gstreamer-audio = "0.25"   # AudioInfo/caps helpers
```
`core` gains `base64 = "0.22"`. MSRV of gstreamer 0.25.4 is 1.92, and the SDK has 1.98.1. Release
builds vendor these like every other crate. The `-sys` crates link against the SDK's pkg-config.

**Pipeline**, built on Begin and set to `Null` on End, error or quit. The `Null` state is what
releases the capture stream, so GNOME's microphone-in-use indicator clears, the counterpart of macOS
`engine.reset()` (M `AudioController.swift:218-224`).

```
# capture branch
pulsesrc client-name="Fermix" buffer-time=40000 latency-time=10000
  ! audioconvert ! audioresample
  ! audio/x-raw,format=S16LE,layout=interleaved,rate=48000,channels=1
  ! webrtcdsp name=dsp probe=echo echo-cancel=true high-pass-filter=true
              noise-suppression=true noise-suppression-level=low gain-control=true
  ! audioconvert ! audioresample
  ! audio/x-raw,format=S16LE,rate=24000,channels=1
  ! appsink name=mic emit-signals=false sync=false max-buffers=8 drop=true

# playback branch (same pipeline, so both share one clock and one lifecycle)
appsrc name=voice is-live=true format=time do-timestamp=true block=false
       caps=audio/x-raw,format=S16LE,layout=interleaved,rate=24000,channels=1
  ! audioconvert ! audioresample
  ! audio/x-raw,format=S16LE,rate=48000,channels=1
  ! webrtcechoprobe name=echo
  ! pulsesink client-name="Fermix" buffer-time=60000 latency-time=10000
```

Design rules:
- **Mic to socket.** The `appsink` callback runs on a GStreamer streaming thread. It appends samples
  to a 100 ms accumulator and hands each full 4 800-byte block **directly** to the socket writer's
  bounded drop-oldest audio queue. It never touches the GTK main loop, for the reason macOS gives
  (M `AudioOwner.swift:122-128`).
- **Two-stage gate,** as on macOS (M `AudioController.swift:158-168`). The callback drops the buffer
  unless *both* "streaming armed" (set on `state:listening`) and "not muted" are true. Keep both in
  one `Arc<AtomicBool>` pair owned by the call, not in a global.
- **Socket to speaker.** Keep a Rust-owned `PlaybackQueue` in `core`: a ring of i16 samples, bounded
  at 30 s, drop-oldest with a log line. Feed `appsrc` from its `need-data` callback in 20 ms
  (480-sample) blocks, and **push zeros when the queue is empty**. The echo probe then always has a
  reference signal, and the live pipeline never underruns into a stall.
  - `playback_stop` clears the queue. The residual audio is bounded by appsrc's queue (set
    `max-bytes` ≈ 40 ms) plus the sink's 60 ms `buffer-time`. Test that audible audio stops within
    about 100 ms.
  - "Drained" means the queue went from non-empty to empty. Fire it after the sink latency, and use
    it to drop the speaking tail (macOS `onPlaybackDrained`).
  - `played_ms` for `interrupt` is the samples dequeued since the utterance anchor, minus the sink
    latency the pipeline reports (`gst::query::Latency`), divided by 24. Precision of ±50 ms is
    enough.
- **Output level for the pet:** compute RMS per `audio_delta` in `core` and smooth it with α = 0.35.
  There is no `level` element and no main-thread hop per sample. Post at most one value per delta to
  the main loop.
- **Warm-up:** set the pipeline to `Playing` on Begin, before `call_start`, with the gate closed.
  PulseAudio stream setup overlaps the provider handshake, as macOS does (M `AudioOwner.swift:95-99`).
- **Errors:** watch the pipeline bus on the main loop (`bus.add_watch_local`). The GStreamer error
  text is for the log. The user sees a sentence from section 4.3. Any bus error ends the call
  (`call_stop`, pipeline to `Null`).
- **Device choice:** the default source and sink, with no `device=` property. The user picks devices
  in GNOME Settings → Sound, as with any GNOME app. Stereo mics (such as the owner's
  `stereo-fallback` AMD mic) are downmixed by `audioconvert`.
- **Do not set `media.role=phone` or `filter.want=echo-cancel`.** On a real PulseAudio host those make
  `module-filter-apply` insert a second echo canceller in front of `webrtcdsp`. Leave the role unset.
- **Bluetooth headsets:** opening a capture stream may switch the headset to the HFP/HSP profile, a
  host policy that lowers playback quality. Document it; do not fight it.

### 3.4 Why not the alternatives

| option | verdict |
|---|---|
| **cpal** | Linux backends are ALSA (reaching Pulse through the runtime's ALSA plugin) or JACK. No AEC; resampling needs `rubato`; a sandbox device model that is harder to reason about. Every piece the app needs would be hand-assembled |
| **libpulse-binding** (+ `webrtc-audio-processing` crate) | Workable, but you rebuild resampling, AEC plumbing and threading that GStreamer already ships in the runtime, and the AEC crate brings a C++ build |
| **pipewire-rs** | The native socket is not in the sandbox (measured), Flatpak clients get restricted PipeWire permissions, and Flathub treats `xdg-run/pipewire-0` as exceptional |
| **Host PipeWire `module-echo-cancel`** | Correct DSP, but it is host configuration the app cannot and must not install. Absent on this host. If a user runs it, they pick the echo-cancelled source as default, and `webrtcdsp` on top is harmless. Mention it only in troubleshooting |

### 3.5 Flatpak finish-args

Current args (L `io.tezra.Fermix.yml`): `--share=ipc`, `--socket=wayland`, `--socket=fallback-x11`,
`--device=dri`, `--filesystem=~/.fermix:ro`, `--talk-name=org.freedesktop.systemd1`.

Add **only**:
```yaml
  # Voice calls: microphone capture and speech playback go through the
  # PulseAudio protocol (pipewire-pulse on current desktops). Flatpak has no
  # finer grant: this socket is both record and play (M38 §7.5).
  - --socket=pulseaudio
```
Do **not** add `--device=all`, `--filesystem=xdg-run/pipewire-0` or any portal talk-name. No build
modules are needed: GStreamer, webrtcdsp and libpulse come from the runtime.

### 3.6 Latency budget (target mouth-to-provider under 150 ms locally)

| stage | budget |
|---|---|
| pulsesrc | about 10–40 ms |
| webrtcdsp | 10 ms frames |
| 100 ms uplink chunk | up to 100 ms |
| socket | under 1 ms |
| playback: appsrc | about 40 ms |
| playback: pulsesink | 60 ms |

Provider latency dominates. If VAD feels sluggish, 50 ms chunks (2 400 bytes) are a one-constant
change.

---

## 4. Prerequisites and readiness

### 4.1 What voice needs

| # | prerequisite | where it is set | cite |
|---|---|---|---|
| 1 | Daemon running, fermix ≥ **0.11.0** (protocol window 1..2) | Home, "Start Fermix" (existing) | 1.2 |
| 2 | `[fermix_core.realtime] enabled = true` | `settings.apply {"section":"realtime","values":{"realtime_enabled":true}}` (golden: E `priv/management/fixtures/requests.jsonl:20`) | E `management/settings/voice.ex:110-116` |
| 3 | `provider = "openai"` (default) | config.toml | E `readiness.ex:423-447` |
| 4 | **OpenAI Platform API key (`sk-…`) in the `openai` provider slot.** A ChatGPT/Codex sign-in does **not** authorise the Realtime API | `secret.set {"id":"openai_api_key","value":"…"}`, the same slot the OpenAI provider uses (E `settings/voice.ex:160-169`) | E `lib/fermix/cli/voice_command.ex:39-58`; E `readiness.ex:456-458` |
| 5 | **Daemon restart** after 2–4. `realtime` is boot-bound ("Voice settings changed since Fermix started.") and so is `providers` | existing systemd Restart (L `app/src/daemon.rs` `UnitVerb::Restart`) | E `setup/restart_state.ex:58-61`; E `application.ex:396-399` |
| 6 | Live engine only (model `gpt-live-1`): a working primary provider for backend delegations. Live bills by the minute even when silent | Providers | E `settings/voice.ex:117-121`, `:144-158`; E `live_session_server.ex:19-21` |
| 7 | A microphone and speakers reachable through the sound server, and the Flatpak `pulseaudio` socket not overridden away | GNOME Settings → Sound; `flatpak override` | 3.1 |

Model, voice, reasoning effort, the "End a conversation after" (1–240 min) and "Stop a conversation
at" (cents) limits, and "Keep transcripts" are ordinary descriptor rows from `settings.get
{"section":"realtime"}` (E `settings/voice.ex:84-91`, `:170-194`). Render them with the generic
settings renderer the Settings slice builds. Do not hand-code a voice form.

### 4.2 How the app reads readiness (no probing, nothing inferred)

- `overview.get` → `realtime`: `{enabled, status: disabled|setup_required|degraded|ready, provider,
  engine, model, socket_alive, active_sessions, active_clients, companion_connected}`
  (E `management/router.ex:840-851`; E `health.ex:222-267`).
- `overview.get` → readiness failures: component `realtime:openai`, pane `voice`, with the
  daemon's action sentence. Render it **verbatim**: "Add the OpenAI API key in Providers settings,
  or turn voice off." / "Set the voice provider to `openai` in config.toml, or turn voice off."
  (E `readiness.ex:441-447`, `:550-551`). The failure is advisory and does not gate setup
  (E `readiness.ex:150-160`).
- Restart state: the `settings.apply` answer and `overview.get` health restart reasons. Render the
  daemon's sentence.
- The socket attempt itself is the final truth: ENOENT, ECONNREFUSED, handshake failure or error.

### 4.3 What the Voice page says, top to bottom

1. **Microphone, then the microphone statement** (M38 §6.5 row at line 2306, required by §7.5
   lines 2789-2827 and §8.6 line 2972). First a property row, **Microphone**, naming the sound
   server's default input as the server names it ("None" when it offers no input but copies of its
   outputs, "Unknown" when the list cannot be read), kept current as devices come and go. Reading
   the list never opens a device. Under it, the statement, verbatim and **above** the Begin button.
   Amended 2026-09-26 (the owner found the page too wordy): the statement is an expander row whose
   title is its first sentence, "Linux has no microphone permission", always in view, and whose
   body is the rest, one click away. Settings → Voice still shows it whole:
   > "Linux has no microphone permission. Nothing asked you, nothing appears in your system settings,
   > and there is nothing to revoke. While Fermix is running it can open the microphone at any time,
   > and so can any other program you run. Your real controls are to not run it, to mute the
   > microphone in your sound settings or in PipeWire, or to run it in a sandbox that withholds
   > audio, which also stops it playing sound. On macOS the operating system asks first. On Linux it
   > does not."

   **No fake permission prompt.** Do not port macOS's "Fermix asks for the microphone the first time
   you begin a voice call" (M `Localizable.strings:361`) or its Permissions ledger "request"
   action. There is nothing to request.

   See D1 in section 5.1 for the Flatpak wording question.
2. **Prerequisite state.** Show exactly one of these rows, in this order:

| condition (source) | sentence | action |
|---|---|---|
| daemon not running (no `daemon.sock` answer) | existing "Fermix is not running." | Start Fermix |
| `realtime.enabled == false` | "Voice is off. Turn it on to talk to Fermix from this app." | the `realtime_enabled` row |
| readiness failure `realtime:openai` | daemon sentence verbatim + "A ChatGPT sign-in does not cover voice; it needs an OpenAI API key." | OpenAI key row (`secret.set openai_api_key`) |
| restart reason for `realtime` or `providers` | daemon sentence verbatim | Restart Fermix |
| `enabled` and `status == degraded`, or `realtime.sock` ENOENT or ECONNREFUSED | "Voice is on, but Fermix has not opened its voice connection. Restarting Fermix usually fixes this." | Restart Fermix |
| handshake `client_too_old` / `client_too_new` | "Update this app to talk to this version of Fermix." / "Update Fermix to talk to this app." | none |
| `max_clients_reached` | "Four other voice clients are already connected to Fermix." | none |
| capture pipeline fails: pulse connection refused or access denied | "Fermix cannot reach the sound server, so it cannot hear or speak. If you removed its sound permission, voice will not work until you restore it." | none |
| the sound server offers no input (before a call, from the device list; and after a call whose capture found none) | "No microphone is connected." | none |
| ready | "Ready" | Begin voice call |

3. Mascot, status word, controls, then the live-call block (caption line, task, "Voice so far: …").
4. The Companion window switch with the honest footer (section 2.4).

The M38 §8.6 row "The companion exists for macOS today, and there is no Linux companion yet."
(M38 line ~2971) must be retired in the same change that ships the first call. Retired 2026-09-26:
Settings → Voice shows the microphone statement alone.

---

## 5. Risks, and the build order

### 5.1 Risks

| # | risk | mitigation |
|---|---|---|
| R1 | **Self-barge-in:** the reply leaks from the speakers into the mic, the server VAD cuts the reply off. Worst in the first seconds, while AEC3 converges | `webrtcdsp` AEC with the always-running echo probe (zeros when idle). Test matrix: laptop speakers plus internal mic (the owner's machine), headphones, USB speakerphone (which does its own AEC). Acceptance: a 60 s answer on laptop speakers is not self-interrupted |
| R2 | 24 kHz is not a `webrtcdsp` rate | Process at 48 kHz, resample after (3.3). A misconfiguration shows up as a caps negotiation failure at `Playing`, so test it in CI inside the SDK |
| R3 | Main-loop stalls delay audio | The audio path never touches GTK: appsink goes to the writer thread, and the reader feeds the playback queue through the main-loop reducer only as control flow. Put the socket in `core` on std `UnixStream` with a reader thread and a writer thread, bounded channels, and `async-channel` into `glib::spawn_future_local` |
| R4 | **Live bills by the minute, even when silent** | Always send `call_stop` on End, window close and quit. Show "Voice so far". The daemon also tears down on EOF and enforces the cost ceiling |
| R5 | Protocol drift between repos | Vendor `priv/realtime` by checksum and run golden-fixture tests in `core`. The daemon ships first (E `PROTOCOL.md:106-122`) |
| R6 | **Engine wire quirks:** Realtime `assistant_text_delta` repeats the full text at the end (E `session_server.ex:876-879`); `transcript_delta` is the whole user turn plus an unlisted `role`; `tool_event` sends `running` and an unlisted `name`; Realtime errors have no `kind`; `usage` shapes are not in the schema | Do not render Realtime transcripts in v1 (macOS does not). Map `running` explicitly. Map errors by `reason` and by `kind`. File the text duplication as an engine issue, not a client workaround |
| R7 | Presence limits: no keep-above, no remembered position, no tray on GNOME | The honest companion window (2.4). The Voice page is the primary surface |
| R8 | Flatpak's audio grant is coarse and may change (`--socket=pulseaudio` may later need justification on Flathub; an audio portal may land) | One finish-arg with a comment. Track xdg-desktop-portal #1129 |
| R9 | Enabling voice needs a daemon restart, which interrupts running work | Reuse the existing restart flow and its warnings. Never restart implicitly on the switch |
| R10 | Voice tools that assume macOS: `screen_share` needs computer use, which is unavailable on Wayland (M38 §8.2) | The daemon refuses at tool level. The client shows `tool_event` `error` with its `reason` |
| R11 | Bluetooth profile switch and device hot-plug mid-call | Bus error ends the call with a sentence. No auto-rebuild in v1 |
| R12 | Stale plan doc: M38 §8.3 says protocol v1 and lists no v2 frames (line 2909) | Update M38 §8.3 and §8.6 when voice ships |
| D1 | **Owner decision:** the §6.5 microphone string predates the Flatpak build. In a Flatpak, "run it in a sandbox that withholds audio" is literally `flatpak override --user --nosocket=pulseaudio io.tezra.Fermix` | Keep the string verbatim. Ask the owner whether to amend the one catalogue row, not add a variant. §6.5 forbids variants |

### 5.2 Build order (each slice ends with a build the owner runs)

- **V0: wire core** (no UI, not shippable, about 1 day).
  - `core/contracts/realtime/` vendored with checksums.
  - `core::realtime` with event types, NDJSON framing with caps, handshake state machine, the pure
    `VoiceState` reducer and `VoiceEffect`s, `PlaybackQueue`, and mascot motion functions.
  - Tests: every golden line; handshake timeout and window; reducer transitions ported from macOS
    `AppModel`; queue drop and flush; `played_ms`.
  - Test the socket client against a `UnixStream` pair playing a scripted daemon.
- **V1: Voice page, prerequisites only** (shippable).
  - Microphone statement, the `realtime` settings rows, the OpenAI key row, restart handling, the
    prerequisite ladder from 4.3, and a disabled "Begin voice call" that gives its reason.
  - The owner can turn voice on and restart from the app. No audio code yet, no `--socket=pulseaudio`.
- **V2: first real call, the smallest voice slice** (shippable).
  - Add `--socket=pulseaudio`, the GStreamer pipeline with AEC, Begin/End, Mute, Stop, the status
    word and icon, and error sentences.
  - Realtime engine. A static listening-pose mascot or a status icon.
  - Acceptance on the owner's laptop, with speakers, internal mic and a 5-minute call:
    - the conversation works;
    - Stop cuts the reply within about 100 ms;
    - no self-interruption on a long answer;
    - End releases the mic, so GNOME's indicator clears (verify on target);
    - killing the app ends the daemon session (the daemon log shows teardown).
- **V3: animated mascot** on the Voice page: layers, crossfade, breath/bob/sway, blink, speaking
  pulse, reduce-motion.
- **V4: companion window**: undecorated, `WindowHandle` drag, secondary-click menu, "Keep on top…"
  through `show_window_menu`, honest footer. Closing the last window ends the call.
- **V5: Live engine extras**: `call_ready`, caption line, task status and cancel, "Voice so far",
  and the `update_required` path.
- **V6: hardening**: device loss and suspend/resume end the call cleanly; the `reconnecting` word;
  voice facts in the diagnostics bundle (engine, model, last error reason, pipeline caps; no audio).
- **Deferred:**
  - push-to-talk through the GlobalShortcuts portal;
  - Realtime transcripts (wait for the engine fix);
  - device pickers;
  - a tray.

---

## 6. Pet artwork

- **Where:** M `Resources/PetExpressions/` holds 15 PNG files:
  - `pet_{idle,listening,thinking,speaking}_{body,face,ring}.png`
  - `pet_{thinking,speaking}_decor.png`
  - `pet_ball.png`

  `pet_idle_decor` and `pet_listening_decor` do not exist, by design (M `Pet/PetAssetCache.swift:236-249`).
- **Format:** every file is a **1024×1024 8-bit RGBA PNG**, one shared registration canvas, about 3.4 MB
  in total. Decoded, all 15 take about **60 MB** of RGBA. Scale to **256×256** at build time. The
  display is 116×108 logical, 232 px at 2× scale. Keep the (−12, +24) face offset in 1024-canvas
  units and scale it.
- **Origin and licence:** first-party. The files were added in fermix commit `3dcce905`
  (2026-05-11, "feat(m9.1): native macOS realtime voice companion") and moved to fermix-macos
  unchanged (`b46298d`, `8852cc5`). The fermix-macos repo is **MIT, © 2026 Tezra** (`LICENSE`), and
  the Linux workspace is MIT too (L `Cargo.toml`). **Reusable as is.** Add a `first_party` record to
  L `app/marks/PROVENANCE.json` (source repo, path, commit, SHA-256 of each PNG), because that file's
  policy requires provenance for every shipped mark. No layered or vector master exists in either
  repo; the PNGs are the masters.
- **Also available:** a one-ink "PetMark" derived from the listening pose by
  `fermix-macos/scripts/build_mascot_mark.py`, used on the macOS Pet page preview
  (M `Design/Components/FermixGlyphs.swift:105-130`). Optional for a symbolic icon. The Linux app
  icon (a blue square with a wordmark glyph) is a separate design and does not change.
- **Packaging:** GResource through `glib-build-tools`, or `install -Dm644` into
  `/app/share/io.tezra.Fermix/pet/`. A missing layer is a packaging defect, and a test must fail the
  build on it, as macOS's `PetSurfaceTests` does (M `Pet/MascotArtwork.swift:59-65`).
