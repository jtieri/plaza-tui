# Session Resume — Plaza TUI

Paste this into a new session to continue development. It captures where we are and
what to do next.

---

## Your role

You're continuing development on **plaza-tui**, a terminal client for
[Nightwave Plaza](https://plaza.one) (vaporwave internet radio), written in Rust.
The codebase is being prepared to go public and is held to a high quality bar.

**Read these first** (they're the source of truth):
- `tasks/roadmap.md` — the living development roadmap (what's done / what's next).
- `tasks/recording-design.md` — design + correctness rules for the recorder feature.
- `tasks/todo.md` — phase history and detail.
- `tasks/lessons.md` — lessons from past corrections.
- `~/development/rust/bobba/docs/style.md` — **the Rust style guide we follow** (Part I
  is universal; obey it). Key rules: dependency-rule crate boundaries, private-by-default,
  `thiserror` per library crate + `anyhow` in the binary, comments explain *why* (never
  restate code or narrate change history), self-documenting code first, TDD.
- Project memory lives at
  `~/.claude/projects/-home-anon-development-rust-plaza-tui/memory/` (see `MEMORY.md`
  index; especially `recording-feature.md`, `plaza-decode-architecture.md`,
  `audio-codec-decisions.md`).

## Working agreement (from the user)

- For weighty decisions, do the research, then give a **decisive recommendation in
  prose** and confirm — don't fire cold multiple-choice prompts.
- **Definition of done = the four gates pass:** `cargo fmt --all --check`,
  `cargo clippy --workspace --all-targets --all-features -- -D warnings`,
  `cargo test --workspace`, and `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`.
  Or just run `just ci`.
- **Git workflow:** branch off `main` for work; commit logically; when a chunk is done,
  open a PR with `gh` and merge it (`gh pr merge N --merge --delete-branch`). The user is
  fine with this. Repo is public: `jtieri/plaza-tui`, `gh` is authenticated, SSH remote
  (a push may occasionally need one retry).
- Verify against reality (runtime logs at `~/.local/share/plaza-tui/plaza-tui.log`, live
  endpoints with `curl`/`ffprobe`) rather than assuming. Don't assert what a crate does
  without checking.

## Current state

- Branch: **`main`**, all four gates green, 53 tests (+7 ignored network tests).
- The repo is a **Cargo workspace**:
  - `crates/plaza-api/` (lib, `thiserror`) — REST client, Socket.IO, auth/keyring, models.
  - `crates/plaza-audio/` (lib, `thiserror`) — `Player`, the codec-agnostic `PcmSource`
    pipeline, MP3/Opus/HLS sources, MPEG-TS demux, `StreamQuality`, and the recorder
    (`src/recording/`). No async-runtime dependency (failures via an `ErrorReporter`
    callback).
  - `crates/plaza-tui/` (bin, `anyhow`) — config, app state + run loop, ratatui UI; thin
    `main.rs`.
- CI in `.github/workflows/ci.yml` (four gates × ubuntu/macos/windows). `justfile` mirrors it.

## What's done

- **Audio fixed + full codec parity.** Plaza moved `/ogg` to **Opus** (symphonia can't
  decode it) which had broken playback; also killed a reconnect storm and fixed `_low`
  URLs. Now MP3 (`/mp3`,`/mp3_low`), Opus (`/ogg`,`/ogg_low`, via libopus through the
  `opus` crate), and HLS/AAC (`/hls`, m3u8 + hand-rolled MPEG-TS demux + symphonia AAC)
  all decode behind one `PcmSource` trait. HLS uses a background-fetch design with
  bounded latency (`select_window`) to avoid drop-outs and unbounded drift.
- **Productionized** to the style guide: workspace split, per-crate errors, full
  public-API docs (`#![warn(missing_docs)]` on libs), clippy-clean, CI, README, dual
  MIT/Apache license.
- **Phase 3 so far:** sleep timer (`t`), favorites export (`e`), and the **stream
  recorder (3a)** — records full Opus songs to a tagged FLAC library.

## The recorder (just finished — 3a)

- Code: `crates/plaza-audio/src/recording/` (`flac.rs` encoder, `recorder.rs` thread +
  sink) + `OpusPcmSource` integration in `sources.rs` + `Player` methods
  (`configure_recording`, `cycle_recording_mode`, `keep_recording`,
  `set_now_playing_artwork`) + binary `[recording]` config + `R`/`s` keys + status
  indicator.
- **Correctness guarantee (non-negotiable, per user):** a saved file is always exactly
  one complete song or it isn't saved. Enforced by exact in-band Ogg boundaries, atomic
  temp→rename writes, discarding the mid-joined first song and any interrupted song.
- Modes: off / cache (rolling last N) / session. `R` cycles, `s` keeps last cached song
  into `<music>/Plaza/<Artist>/<Album>/<Artist> - <Title>.flac`. In-band tags + embedded
  cover art. **Opus-only** (only stream with exact boundaries).
- **Gotcha already handled:** `flacenc` writes a STREAMINFO declaring variable blocking
  (min≠max) but fixed-blocking frames, which symphonia rejects ("end of stream");
  `flac.rs` patches `min_block_size = max_block_size`. Keep this in mind for 3c playback.
- Validated end-to-end: `cargo test -p plaza-audio --lib records_a_live_song_to_a_valid_flac -- --ignored`
  records a real song and asserts a valid decodable FLAC (~2 min run).

## Verified facts about Plaza (don't re-derive)

- Endpoints: `/mp3` (128k MP3), `/mp3_low` (96k MP3), `/ogg` (64k Opus), `/ogg_low`
  (96k Opus), `/hls` (AAC-LC in MPEG-TS, 3 bitrates). Low-quality paths use an
  underscore, not `/low`.
- `/ogg` is **chained Ogg: one logical stream per song**, each carrying `OpusTags`
  (artist/album/title) in-band — this is what makes lossless recording exact.
- REST is `api.plaza.one/v2/...`; Socket.IO is `plaza.one` path `/ws`. Both already work.
- Plaza is **live radio**: no on-demand catalog, no full-track download API. The recorder
  is what enables an on-demand local library and real playlists.
- HLS metadata leads the audio by the buffer depth (~8s) — tracked as open GitHub
  **issue #3** (not a bug, an enhancement; fix would delay the now-playing display by the
  buffer depth).

## Next steps (pick up here)

Per `tasks/roadmap.md`, the recorder unlocks the on-demand features the user most wants:

1. **3c — Local library + on-demand playback (recommended next, high impact):**
   - Add a local-file `PcmSource` (symphonia decodes our FLAC — the STREAMINFO fix makes
     this work) so `Player` can play library tracks on demand.
   - A library index + a TUI browser view to browse/search recorded songs.
   - This is the foundation for **real playlists** (3d), the user's original ask.
2. **3b — MP3 recording** via icecast inline metadata (`Icy-MetaData` / `StreamTitle`),
   to support recording on the MP3 streams too (exact boundaries there as well).
3. **3d — Local playlists** (build/save/play from the library).
4. Smaller parity/UX items between phases: news "latest/unread" badge; account
   register/profile edit/password/delete; volume OSD overlay; header marquee for long
   titles.

## How to work

- Build/run: `cargo run -p plaza-tui` (needs an audio device + a terminal; the user runs
  it, you can't drive the interactive TUI). Headless verification: the ignored network
  smoke tests in `crates/plaza-audio/tests/stream_smoke_test.rs` and the recorder e2e
  test decode live streams to PCM/FLAC.
- `just ci` before every commit. Keep comments professional and human-authored in voice;
  describe current state, not how the code got here.
- Keybindings live in `crates/plaza-tui/src/tui/views/help.rs` (keep it in sync when
  adding keys). Status bar + run loop are in `crates/plaza-tui/src/app.rs`.

Start by reading `tasks/roadmap.md` and confirming the four gates pass on `main`, then
propose a plan for 3c (or whichever direction the user wants) before implementing.
