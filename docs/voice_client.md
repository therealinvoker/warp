# Voice client — durable notes

Non-obvious findings about the Bang voice overlay (hands-free voice in/out). Read
this before touching the overlay, TTS, transcription, or the spoken-text
normalizer. Companion to [`design.md`](design.md) (visual UI notes).

## Architecture: a 3-stage pipeline

Voice "in and out" is **three separate stages**, not one model:

1. **Speech-to-text (STT)** — OpenAI Realtime transcription, proxied by the
   harness (`../harness-backend/src/ai/realtimeRelay.js`). The client streams mic
   audio over a WebSocket to `/ai/realtime`.
2. **The brain (reasoning + tool calls)** — the normal Bang agent turn
   (`harness-backend/src/routes/ai.js` → OpenAI/Bang AI). Produces the text
   answer.
3. **Text-to-speech (TTS)** — local Piper neural TTS (`crates/voice_tts`),
   played in-app.

Implication: a plain text LLM (Gemma, Qwen, Llama, …) can only ever be **stage 2,
the brain**. It cannot do STT or TTS. A single-model "voice in/out" would require
a speech-to-speech model (OpenAI Realtime, Gemini Live, local Moshi/Qwen-Omni),
which is a different design from the one we run.

## Where things live

| Concern | Location |
| --- | --- |
| Overlay window, pucks, settings popover (native) | `app/src/overlay/native/*.m` |
| Overlay Rust bindings / FFI callbacks | `app/src/overlay/platform_mac.rs`, `mod.rs`, `platform.rs` |
| Voice event wiring, spoken-text normalization | `app/src/terminal/input.rs` |
| Local TTS engine (Piper) + software AEC | `crates/voice_tts/` |
| Settings (values) | `app/src/settings/ai.rs` (`voice_overlay_*`, `agent_verbosity`) |
| Settings UI (sliders/toggles) | `app/src/settings_view/ai_page.rs` |
| Realtime STT relay, VAD, auto-submit | `../harness-backend/src/ai/realtimeRelay.js`, `config.js` |
| Verbosity → system prompt | `../harness-backend/src/routes/ai.js`, `src/proto/index.js` |

## STT / Realtime relay

- **Pin the language.** Short/ambiguous utterances (e.g. "hey") get mis-detected
  as another language (Chinese was the repro). The client sends the
  `voice_overlay_language` setting as a `?language=` query param on the
  `/ai/realtime` WebSocket; the relay passes it to the transcription session
  (empty = auto-detect). Configurable in Settings → AI **and** the overlay gear.
- **Auto-submit hinges on one event mapping.** Map OpenAI's GA
  `conversation.item.done` → `transcript.done` + `turn.end` in `realtimeRelay.js`.
  The older `conversation.item.input_audio_transcription.completed` may never
  fire, so relying on it alone means dictation never auto-submits. A post-submit
  `accepting` gate dedupes if both arrive.
- **VAD tuning** (`config.js`): `vadThreshold` (lower = easier barge-in),
  `vadSilenceMs` (longer = pauses/thinking don't submit early — 1200ms is a good
  balance), `vadPrefixMs`. Threshold and silence are independent — keep threshold
  low for barge-in while keeping silence generous.
- **One tab owns the mic.** Only the tab that opened the overlay (the
  `OverlayController::active_input`) processes voice events — otherwise transcript
  writes fan out to every tab and auto-submit fires multiple times. Focus-follow
  moves ownership to the newly focused tab.

## TTS (Piper, local)

- **Why local Piper, not `say` or a cloud voice:** consistent voice across
  machines, no per-utterance cost, and — critically — we own the PCM so it can be
  echo-cancelled for barge-in. Engine: ONNX Runtime (`ort`) + `ndarray`.
- **GPL isolation.** Piper needs eSpeak-NG phonemes, and eSpeak-NG is GPL. We
  invoke it strictly **out-of-process** (a bundled `espeak-ng` binary) behind the
  `Phonemizer` trait, so the shippable app never links GPL code. Keep it that way.
- **Preload off-thread on overlay open** (`preload_tts`) so the first spoken
  answer doesn't stall while the model loads.
- **We do not listen while speaking.** The mic would transcribe our own TTS.
  Playback is `AVAudioPlayer` (`tts.m`); when an utterance finishes on its own it
  calls `bang_tts_did_finish` → resume listening. A stop/interrupt/replacement
  does **not** fire that callback.
- **Sentence pauses are inserted manually** (`crates/voice_tts/src/lib.rs`,
  `split_for_pauses`). espeak's default clause gap is too short, so we split the
  answer at `.`/`!`/`?`/newline/dashes, synthesize each segment, and stitch with
  real silence (`SENTENCE_PAUSE_MS` ≈ 320ms, `DASH_PAUSE_MS` ≈ 240ms). These two
  constants are the pacing knobs.

## Spoken-text normalization (`strip_markdown_for_speech`)

espeak reads many symbols as a **breathy clause-break** or a literal symbol name.
Normalize the answer to words **before** synthesis (this only affects spoken
audio, never the on-screen text). Current rules, all in `strip_markdown_for_speech`
in `app/src/terminal/input.rs`:

- Strip markdown: `*` `` ` `` `#`, and `[label](url)` → `label`.
- Skip emoji / pictographs (`is_speech_skippable_symbol`) so TTS doesn't say
  "thumbs up sign".
- `/` → "slash" (paths like `/Users/foo` otherwise breathe).
- `-` → "dash" for separators/flags/ranges, **but keep an intra-word hyphen**
  (`well-known` stays natural).
- `.` → "dot" for an in-token dot (`file.jpg`, `v1.2.3`, `example.com`), but a
  sentence-ending `.` stays a silent pause ("Done." is not "Done dot").
- digit-group `,` **dropped** so `11,200` reads "eleven thousand two hundred"
  (a comma otherwise becomes a pause); prose commas kept.
- `~` → "around" (approximation, `~2`) or "home" (`~/Users`).
- `%` → "percent", `&` → "and", `=` → "equals", `>`/`<` → "greater/less than".
- `$` currency-aware: `$1,200` → "1200 dollars" (unit spoken **after** the
  amount, commas dropped); `$PATH` → "dollar PATH".
- `push_spoken_word` helper guarantees the inserted word is space-separated so it
  never glues onto neighbors (`foo/bar` → "foo slash bar").

When a new symbol sounds wrong, it's almost always a one-line addition to this
match. Prefer deterministic word replacement over relying on espeak's
locale-dependent number/currency normalization.

## Acoustic echo cancellation / barge-in (current state)

- **Hands-free barge-in is currently DISABLED** because it self-interrupts (the
  AEC didn't cancel our own TTS reliably enough). Instead we **suppress listening
  while speaking** and support **tap-to-interrupt** (tap the mic puck). The AEC
  infrastructure is kept for a later on-device tuning pass — do not delete it.
- Software AEC (`crates/voice_tts`, feature `aec`, WebRTC APM) runs at a fixed
  48kHz in 10ms frames; mic (24kHz) and TTS (22.05kHz) are resampled in/out. Feed
  TTS PCM as the far-end **reference paced to a monotonic clock** (not a fixed
  sleep) so AEC3's adaptive delay estimator can lock on.
- macOS `AVAudioEngine` voice-processing path (`aec_capture.m`): the duplex I/O
  only pulls the mic when its render side is active, so we render a **dedicated
  silence source** into the output. Wiring the mic → mainMixer instead makes the
  mic its own AEC reference (self-cancels to silence); and the silence source must
  connect straight to `outputNode` at the input node's VP format, or unit init
  fails with `-10875`.

## Response verbosity (0–10)

- Setting `agent_verbosity` (0 = terse "Done.", 10 = exhaustive). Sliders live in
  Settings → AI **and** the overlay gear popover — both write the same setting.
- **Transport with no client-proto change:** the client proto is an external
  generated crate (`bang-proto-apis`), so we can't add a `Settings` field. Instead
  the value rides in the existing `Request.metadata.logging` map (a generic
  `string → google.protobuf.Value`) under `response_verbosity`. The harness adds
  the matching `logging` field to its `Metadata` proto, reads it
  (`extractVerbosity`), and appends `verbosityDirective(n)` to the system prompt.
  The directive scales **prose length/detail only** — it must never license
  skipping tool calls or safety checks.

## Interrupting a working agent

Barge-in / stop must fully stop the agent (including command **monitoring**),
using the same `handle_ctrl_c` path as the stop button — not drop into
steering/suggestion mode. `overlay_agent_snapshot` treats monitoring/driving as
"thinking" so dictation stays suppressed until the agent is truly idle.

## Harness dev gotcha

`npm run dev`/`start` run `scripts/free-harness.mjs` first (`predev`/`prestart`)
to kill any stale harness holding port 8088 — otherwise an old process keeps
serving old code. Editing `.proto` requires a harness reload (a JS edit triggers
`node --watch`, which re-reads the proto).
