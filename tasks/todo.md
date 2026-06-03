# Task: Plaza TUI — Go-Public Roadmap (Parity + Productionization)

> Goal: fix the broken audio + input, achieve 100% codec/stream parity with plaza.one,
> productionize the codebase (clippy-clean, tested, CI), then build beyond-parity features.

## Diagnosis (confirmed 2026-06-02)

- **Audio dead**: Plaza switched `/ogg` from Vorbis → **Opus** (verified live: `/ogg`=64k Opus,
  `/ogg_low`=96k Opus). Symphonia has **no Opus decoder** → every track fails
  `unsupported codec`. Runtime log: 273/273 opus mappers failed today; 580 vorbis decoded fine
  before Apr 27. Root cause is upstream, not our refactor.
- **Reconnect storm**: new `Live` mode retries forever on the *permanent* "unsupported codec"
  error (~1 reopen/sec), which also degrades UI responsiveness (likely the "arrow keys" symptom).
- **Stale endpoints**: config uses `/mp3/low` & `/ogg/low` (slash) → **404**. Correct paths are
  `/mp3_low` & `/ogg_low` (underscore), both live (96k).
- **HLS**: `/hls` = master playlist → 3 AAC-LC variants in **MPEG-TS** (`.ts`). Symphonia has an
  AAC decoder but **no MPEG-TS demuxer**. `hls.rs` is currently dead code.
- **Net**: only `/mp3` & `/mp3_low` are decodable today. Opus + HLS need new decode paths.

## Confirmed codec/stream matrix (full parity target)

| Quality        | Endpoint    | Container/Codec   | Decoder needed                    |
|----------------|-------------|-------------------|-----------------------------------|
| MP3 128k       | `/mp3`      | MP3               | symphonia (have it)               |
| MP3 96k        | `/mp3_low`  | MP3               | symphonia (have it)               |
| Opus 64k       | `/ogg`      | Ogg / Opus        | ogg demux (symphonia) + libopus   |
| Opus 96k       | `/ogg_low`  | Ogg / Opus        | ogg demux (symphonia) + libopus   |
| HLS lo/mid/hi  | `/hls`      | MPEG-TS / AAC-LC  | m3u8 + TS demux + symphonia AAC    |

## Plan

### Phase 0 — Stop the bleeding (audio works again, minimal risk) — DONE 2026-06-03
- [x] Fix stream URLs: `/mp3_low`, `/ogg_low` (underscore); removed stale `Hls`→`Ogg` migration.
- [x] Treat decoder-init / decoder-reset failure as **permanent** (`SessionOutcome::Permanent`):
      no reconnect; reports a clear message via the (now-wired) audio error channel to the UI.
- [x] Default `stream_quality = Mp3`; saved non-decodable prefs (e.g. user's `ogg`) fall back to
      MP3 at runtime with a notification, without overwriting the stored preference.
- [x] Input hardening: ignore `KeyEventKind::Release` events in the event loop.
- [x] Tests: permanent-error⇒no-retry + reports error; transient⇒retry; `_low` URL mapping;
      `is_supported` matrix; live MP3 decode smoke test (ignored/network).

### Phase 1 — Codec parity (the core ask)
- [ ] Refactor audio into a codec-agnostic layer: a `PcmSource` that yields
      `(samples: Vec<f32>, rate, channels)` chunks; the existing sink-feed/backpressure/command
      loop is shared by all sources.
  - [ ] `SymphoniaPcmSource` — MP3 (+ keeps Vorbis working via symphonia-all).
  - [ ] `OpusPcmSource` — symphonia Ogg demux for packets → **libopus** decode (`audiopus`,
        building libopus from source so end users need no system package, only a C toolchain).
  - [ ] `HlsPcmSource` — `m3u8-rs` playlist + sliding media playlist poll + minimal MPEG-TS
        audio demux → ADTS → symphonia AAC. Rework `hls.rs` into this.
- [ ] Wire all 5 qualities through `StreamQuality`; HLS bitrate select (auto/lo/mid/hi).
- [ ] Tests: TS demuxer (fixture segment), m3u8 parse, Opus packet decode smoke, source dispatch.

### Phase 2 — Productionize
- [ ] Kill dead code & clippy warnings (target `clippy -D warnings`); remove unused error variants.
- [ ] Consistent error surfacing to the UI; no silent failures.
- [ ] `cargo fmt`, doc comments on public items.
- [ ] **CI** (GitHub Actions): fmt check, clippy -D warnings, build, test on stable; cache;
      install C toolchain for libopus vendored build.
- [ ] Expand tests for regression safety; aim to cover each module.
- [ ] README: features, install (incl. audio backend notes), keybindings, config, screenshots.

### Phase 3 — Beyond parity (after parity lands; scope per user)
- [ ] Remaining API parity: favorites **export**, register, profile edit / password / delete,
      news "latest/unread" badge, REST `/v2/status` seed/fallback.
- [ ] Sleep timer (client-side), volume OSD, header marquee.
- [ ] **New**: local playlists, track/preview downloads (user-requested future work).

## Open decisions for user sign-off
1. Opus via **libopus built-from-source** (no runtime system dep; needs C toolchain at build).
2. HLS: implement real MPEG-TS/AAC (chosen, per parity goal) — reworks `hls.rs`.
3. Sequencing: land Phase 0 first (audio back fast), then 1 → 2 → 3.

---

## Completed Work

### Phase 1: Scaffolding — DONE
- [x] Cargo.toml with all dependencies
- [x] Module skeleton files matching project structure
- [x] error.rs: PlazaError, AuthError, ApiError, AudioError enums
- [x] main.rs: clap CLI (--reset-auth, --stream-quality, --log-level), tracing to file, panic hook

### Phase 2: Config & Auth — DONE
- [x] config.rs: Config struct (stream_quality, volume, image_protocol), TOML load/save, HLS->OGG auto-migration
- [x] api/models.rs: Song, User, StatusResource, LoginForm/Response, Paginated<T>, FavoriteEntry, HistoryEntry, RatingEntry, NewsItem, UserStats, SongResource, RatingRange
- [x] api/mod.rs: ApiClient with reqwest, auth headers, base URL
- [x] auth.rs: login/logout, keyring save/load/delete token

### Phase 3: REST API Client — DONE
- [x] api/client.rs: All 11 API methods (status, history, song, favorites CRUD, reactions, ratings, user/stats, news)
- [x] Error mapping: 401->Unauthorized, 429->RateLimited, 404->NotFound

### Phase 4: Socket.io — DONE
- [x] socket.rs: SocketClient with broadcast channel
- [x] Events: status, listeners, reactions, disconnect/reconnect
- [x] Exponential backoff reconnection (1s-30s)

### Phase 5: Audio Player — DONE
- [x] audio/hls.rs: Master/media playlist fetch, segment streaming, dedup, retry
- [x] audio/player.rs: Symphonia decode + rodio output, chained Ogg stream support
- [x] play/pause/volume/stop controls
- [x] Non-fatal audio init (app works without audio device)

### Phase 6: TUI Framework — DONE
- [x] theme.rs: Vaporwave palette (9 colors, style helpers)
- [x] tui/mod.rs: Terminal setup/restore (raw mode, alternate screen)
- [x] tui/events.rs: AppEvent enum, EventHandler merging crossterm + tick + socket + audio
- [x] tui/layout.rs: Header (3) + sidebar (18) + content + status bar (1)
- [x] app.rs: AppState, PaginatedState<T>, 1200+ line run loop with async coordination

### Phase 7: Views — DONE
- [x] Login: ASCII logo, username/password form, guest mode
- [x] Now Playing: artwork (sixel/kitty/iterm2 + fallback), artist/title/album, progress gauge, controls
- [x] History: paginated scrollable list, auto-load next page, favorite from list
- [x] Favorites: list with delete support, auth-gated
- [x] Charts: 3-tab ratings (All Time/Weekly/Monthly), ranked list
- [x] News: HTML stripping, word-wrapped items
- [x] Profile: user info, stats, logout
- [x] Help: keyboard shortcuts overlay popup
- [x] Widgets: vaporwave_block, song_detail_popup with preview, centered_rect

### Phase 8: Tests — DONE
- [x] tests/api_models_test.rs: 12 tests (all struct deserialization)
- [x] tests/config_test.rs: 7 tests (round-trip, defaults, quality URLs)
- [x] tests/hls_test.rs: 7 tests (URL resolution, playlist parsing)

### Phase 9: Polish — ~95% DONE
- [x] Ctrl+C graceful shutdown (player stop, terminal restore)
- [x] Timed notification system (3s auto-dismiss)
- [x] Connection status indicator (LIVE/DISCONNECTED/CONNECTING)
- [x] --reset-auth CLI flag
- [x] Artwork async fetch + in-memory cache
- [x] Non-fatal audio/socket degradation
- [ ] Scrolling marquee for long song titles in header
- [ ] Volume OSD overlay (currently uses notification bar instead)
- [ ] README.md with install instructions, features, keybindings, screenshot

## Plan — Remaining Polish Items
- [ ] Evaluate: marquee ticker for header song title
- [ ] Evaluate: volume OSD overlay vs current notification approach
- [ ] README.md content
- [ ] Verification: cargo build --release with no warnings
- [ ] Verification: cargo test — all 26 pass
- [ ] Verification: cargo clippy -- -D warnings clean

## Progress Notes
2026-03-06 — Full codebase audit completed. All phases 1-8 fully implemented. Phase 9 at ~95%. No stubs or TODOs remain in source code.

## Review
<!-- Summary when complete -->
