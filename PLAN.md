# Plaza TUI — Implementation Plan

## Context

Nightwave Plaza (https://plaza.one/) is a Vaporwave radio website with a retro desktop aesthetic.
This project builds a Rust TUI application that replicates the full feature set of the site inside
a terminal, including live audio playback, song history, favorites, charts, reactions, and news.

### Design decisions
- **Audio**: Pure Rust HLS implementation (m3u8-rs + symphonia + rodio) — zero system dependencies
- **Artwork**: Terminal image protocols (sixel/kitty/iterm2) with Unicode block-art fallback
- **Auth tokens**: System keyring (libsecret on Linux, Keychain on macOS) via the `keyring` crate
- **Default stream**: HLS adaptive (`radio.plaza.one/hls`)
- **Theme**: Vaporwave aesthetic — hot pink, cyan, purple, dark navy

---

## API Reference Summary

| Layer | URL |
|---|---|
| REST API | `https://api.plaza.one` (v2 prefix) |
| Audio streams | `https://radio.plaza.one/hls` (HLS master playlist) |
| Real-time | Socket.io at `wss://plaza.one`, path `/ws` |

**Auth**: Bearer token from `POST v2/auth/token {username, password, remember}`.

**Socket.io events**: `status` (StatusResource), `listeners` (number), `reactions` (number).

---

## Project Structure

```
plaza-tui/
├── Cargo.toml
├── PLAN.md
├── README.md
├── src/
│   ├── main.rs              # Entry point, CLI args (clap), bootstrap
│   ├── app.rs               # AppState enum + central event loop
│   ├── config.rs            # Config struct, TOML read/write, keyring token storage
│   ├── auth.rs              # Login/logout, token lifecycle
│   ├── error.rs             # PlazaError enum (thiserror)
│   ├── theme.rs             # Vaporwave color palette + ratatui Style helpers
│   ├── api/
│   │   ├── mod.rs           # Shared reqwest client, base URL, auth headers
│   │   ├── models.rs        # Serde structs (Song, User, StatusResource, etc.)
│   │   └── client.rs        # All API method implementations
│   ├── socket.rs            # Socket.io client (rust-socketio), sends events to app via channel
│   ├── audio/
│   │   ├── mod.rs           # Public API: Player struct
│   │   ├── hls.rs           # HLS: fetch master playlist → select quality → segment loop
│   │   └── player.rs        # Symphonia decode pipeline → rodio output sink
│   └── tui/
│       ├── mod.rs           # Terminal setup/teardown (crossterm)
│       ├── events.rs        # Keyboard + resize event handling
│       ├── layout.rs        # Root layout (header / content / status bar)
│       ├── widgets.rs       # Reusable styled components (vaporwave borders, etc.)
│       └── views/
│           ├── mod.rs       # View enum + render dispatch
│           ├── login.rs     # Username/password form
│           ├── now_playing.rs  # Song info, artwork, progress, listeners, reactions
│           ├── history.rs   # Paginated play history
│           ├── favorites.rs # User favorites list, add/remove
│           ├── charts.rs    # Ratings (overtime / weekly / monthly tabs)
│           ├── news.rs      # Site news
│           ├── profile.rs   # User info + stats
│           └── help.rs      # Keyboard shortcuts overlay
├── tests/
│   ├── api_models_test.rs   # Deserialize fixture JSON → assert fields
│   ├── config_test.rs       # Read/write config file in temp dir
│   └── hls_test.rs          # Parse sample .m3u8 playlists
└── tasks/
    ├── todo.md
    └── lessons.md
```

---

## Cargo.toml Dependencies

```toml
[dependencies]
# TUI
ratatui          = "0.29"
crossterm        = { version = "0.28", features = ["event-stream"] }
ratatui-image    = "2"           # sixel/kitty/iterm2 image rendering

# Async
tokio            = { version = "1", features = ["full"] }
tokio-stream     = "0.1"

# HTTP
reqwest          = { version = "0.12", default-features = false, features = ["json", "stream", "rustls-tls"] }

# JSON / config
serde            = { version = "1", features = ["derive"] }
serde_json       = "1"
toml             = "0.8"

# Socket.io
rust-socketio    = { version = "0.6", features = ["async"] }

# HLS
m3u8-rs          = "6"

# Audio decoding
symphonia        = { version = "0.5", features = ["aac", "mp3", "ogg", "isomp4"] }

# Audio output
rodio            = { version = "0.19", default-features = false, features = ["symphonia-all"] }

# Secure storage
keyring          = "3"

# Error handling
anyhow           = "1"
thiserror        = "1"

# Utilities
clap             = { version = "4", features = ["derive"] }
dirs             = "5"
chrono           = { version = "0.4", features = ["serde"] }
tracing          = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
bytes            = "1"
image            = "0.25"    # For decoding artwork JPEGs before rendering

[dev-dependencies]
wiremock         = "0.6"     # HTTP mocking for integration tests
tempfile         = "3"       # Temp dirs for config tests
```

---

## Implementation Phases

---

### Phase 1 — Project Scaffolding

**Goal**: Compilable skeleton with proper module structure and logging.

1. `cargo new plaza-tui --bin` — initialize project
2. Add all dependencies to `Cargo.toml`
3. Create empty module files for all paths in the project structure
4. Implement `error.rs` — `PlazaError` enum covering Auth, Api, Audio, Io, Config variants
5. Set up `tracing-subscriber` in `main.rs` writing to `~/.local/share/plaza-tui/plaza-tui.log` (never stdout — would corrupt TUI)
6. Add `clap` CLI struct in `main.rs`: flags `--reset-auth`, `--stream-quality [hls|ogg|mp3]`, `--log-level`
7. Create `tasks/todo.md` and `tasks/lessons.md`
8. Verify: `cargo build` succeeds with no errors

---

### Phase 2 — Config & Authentication

**Goal**: User can log in; token persists between sessions.

#### 2.1 Config
1. Define `Config` struct in `config.rs`:
   ```rust
   pub struct Config {
       pub stream_quality: StreamQuality,  // enum: Hls, Ogg, OggLow, Mp3, Mp3Low
       pub volume: f32,                    // 0.0..1.0
       pub image_protocol: Option<String>, // None = auto-detect
   }
   ```
2. Implement `Config::load()` — reads `~/.config/plaza-tui/config.toml`, creates default if absent
3. Implement `Config::save()` — writes TOML back to disk
4. Unit test: write config to temp dir, reload, assert equality

#### 2.2 API Models
5. Define all serde structs in `api/models.rs`:
   - `Song { id, artist, album, title, length, artwork_src, artwork_sm_src, preview_src }`
   - `User { id, username, email, created_at }`
   - `StatusResource { song, listeners, updated_at, reactions, position }`
   - `LoginForm`, `LoginResponse { data: User, token: String }`
   - `Paginated<T> { meta: PaginationMeta, data: Vec<T> }`
   - `PaginationMeta { current_page, last_page, per_page, total }`
   - `FavoriteEntry { id, song: Song, created_at }`
   - `HistoryEntry { played_at, song: Song }`
   - `RatingEntry { song: Song, likes: u32 }`
   - `NewsItem { id, text, author, created_at }`
   - `UserStats { reactions, favorites }`
6. Unit tests: parse fixture JSON strings for each struct, assert key fields

#### 2.3 API Client Setup
7. In `api/mod.rs` create `ApiClient` struct wrapping `reqwest::Client` with:
   - base URL `https://api.plaza.one`
   - `Authorization: Bearer <token>` header (when authenticated)
   - `Content-Type: application/json`
8. Implement constructor `ApiClient::new(token: Option<String>) -> Self`

#### 2.4 Auth
9. In `auth.rs`, implement `login(client: &ApiClient, username, password) -> Result<String>`:
   - POST `v2/auth/token` with `{username, password, remember: true}`
   - Returns the bearer token string
10. Implement `save_token(token: &str)` — stores in OS keyring under service `plaza-tui`, account `auth-token`
11. Implement `load_token() -> Option<String>` — retrieves from keyring
12. Implement `delete_token()` — removes from keyring (logout)
13. Implement `logout(client: &ApiClient)` — calls `POST v2/auth/logout` then `delete_token()`

---

### Phase 3 — REST API Client

**Goal**: Full API coverage for all app features.

Implement in `api/client.rs` as async methods on `ApiClient`:

1. `get_status() -> Result<StatusResource>` — GET `/status`
2. `get_history(page: u32) -> Result<Paginated<HistoryEntry>>` — GET `v2/history?page=N`
3. `get_song(id: &str) -> Result<SongResource>` — GET `v2/songs/{id}` (includes `current_user` favorite status)
4. `get_favorites(page: u32) -> Result<Paginated<FavoriteEntry>>` — GET `v2/users/me/favorites`
5. `add_favorite(song_id: &str) -> Result<FavoriteEntry>` — POST `v2/users/me/favorites` with `{song_id}`
6. `remove_favorite(favorite_id: u32) -> Result<()>` — DELETE `v2/users/me/favorites/{id}`
7. `send_reaction(reaction: u8) -> Result<u32>` — POST `v2/reactions`, returns updated count
8. `get_ratings(range: RatingRange, page: u32) -> Result<Paginated<RatingEntry>>` — GET `v2/ratings/{range}`
9. `get_me() -> Result<User>` — GET `v2/users/me`
10. `get_my_stats() -> Result<UserStats>` — GET `v2/users/me/stats`
11. `get_news(page: u32) -> Result<Paginated<NewsItem>>` — GET `v2/news`

Error handling: map 401 → `PlazaError::Auth(Unauthorized)`, 429 → `PlazaError::Api(RateLimited)`.

---

### Phase 4 — Socket.io Real-time Client

**Goal**: App state automatically reflects current song changes without polling.

1. In `socket.rs`, implement `SocketClient` struct with a `tokio::sync::broadcast::Sender<SocketEvent>`
2. Define `SocketEvent` enum: `Status(StatusResource)`, `Listeners(u32)`, `Reactions(u32)`, `Disconnected`, `Reconnected`
3. Connect via `rust-socketio` async client:
   - URL: `https://plaza.one`
   - Socket.io path: `/ws`
4. Register handler for `status` event → deserialize JSON → broadcast `SocketEvent::Status`
5. Register handler for `listeners` event → broadcast `SocketEvent::Listeners`
6. Register handler for `reactions` event → broadcast `SocketEvent::Reactions`
7. Handle disconnect → broadcast `SocketEvent::Disconnected`, reconnect with exponential backoff (1s, 2s, 4s, max 30s)
8. Expose `SocketClient::subscribe() -> broadcast::Receiver<SocketEvent>`

---

### Phase 5 — HLS Audio Player

**Goal**: Live radio stream plays via pure Rust. No system dependencies required.

#### 5.1 HLS Fetcher (`audio/hls.rs`)
1. `fetch_master_playlist(url: &str) -> Result<MasterPlaylist>` — GET the HLS URL, parse with `m3u8-rs`
2. `select_stream(playlist: &MasterPlaylist) -> &VariantStream` — pick highest bandwidth variant
3. `fetch_media_playlist(url: &str) -> Result<MediaPlaylist>` — GET the selected variant URL, parse
4. `resolve_segment_url(base: &str, segment: &MediaSegment) -> String` — handle relative URLs
5. `fetch_segment(url: &str) -> Result<Bytes>` — download a single audio segment
6. Implement `HlsSegmentStream` — async stream yielding `Bytes`:
   - Tracks already-fetched segment URIs to avoid duplicates
   - Re-fetches media playlist on each poll; discovers new segments
   - Respects `#EXT-X-TARGETDURATION` for polling interval
   - Live stream aware (no `#EXT-X-ENDLIST`)

#### 5.2 Audio Decoder + Player (`audio/player.rs`)
7. Implement `Player` struct with `rodio::OutputStream`, `rodio::Sink`, background task handle
8. `Player::start()`:
   - Spawn async task driving `HlsSegmentStream`
   - For each segment: wrap `Bytes` in `Cursor`, feed to `symphonia::core::io::MediaSourceStream`
   - Create `FormatReader` (auto-detect MPEG-TS / AAC)
   - Decode all packets → convert to `SamplesBuffer<f32>` → append to `Sink`
9. `Player::pause()` / `resume()` via `Sink::pause()` / `Sink::play()`
10. `Player::set_volume(f32)` via `Sink::set_volume()`
11. `Player::stop()` — abort HLS task, clear sink
12. On segment fetch failure: retry 3× → if still failing, switch to `/ogg` fallback stream + notify app
13. Accept `StreamQuality` from config to determine which stream URL to use

---

### Phase 6 — TUI Framework

**Goal**: Responsive TUI with Vaporwave aesthetic, navigation, and async event loop.

#### 6.1 Theme (`theme.rs`)
1. Define color constants:
   - `BACKGROUND`: `Color::Rgb(10, 5, 25)` (dark navy)
   - `PINK`: `Color::Rgb(255, 0, 128)` (hot pink)
   - `CYAN`: `Color::Rgb(0, 255, 255)`
   - `PURPLE`: `Color::Rgb(153, 0, 255)`
   - `LAVENDER`: `Color::Rgb(180, 130, 255)`
   - `TEXT`: `Color::Rgb(220, 210, 255)`
   - `DIM`: `Color::Rgb(80, 70, 120)`
2. Style helpers: `title_style()`, `border_style()`, `selected_style()`, `dim_style()`, `highlight_style()`
3. Use `ratatui::symbols::border::DOUBLE` for main panels

#### 6.2 Terminal Setup (`tui/mod.rs`)
4. `setup_terminal()` → raw mode, alternate screen, hide cursor
5. `restore_terminal()` → undo above

#### 6.3 Event Handling (`tui/events.rs`)
6. `AppEvent` enum: `Key(KeyEvent)`, `Resize(u16, u16)`, `Tick`, `StatusUpdate(StatusResource)`, `ListenersUpdate(u32)`, `ReactionsUpdate(u32)`, `AudioError(String)`
7. `EventStream` merges: crossterm events, 250ms tick interval, socket broadcast receiver, audio error channel

#### 6.4 Layout (`tui/layout.rs`)
8. Three vertical chunks: header (3 lines), content, status bar (1 line)
9. Content splits into left sidebar (16 cols, navigation menu) + right main panel
10. Navigation: `[▶ Now Playing] [History] [Favorites] [Charts] [News] [Profile]`

#### 6.5 App State (`app.rs`)
11. `AppState` struct with: current view, now_playing, is_playing, volume, connection status, per-view `PaginatedState<T>`, current user, timed notification
12. `PaginatedState<T>`: items, page, last_page, selected index, loading flag
13. Central `run()` loop: render → await event → dispatch handler → repeat

---

### Phase 7 — View Implementations

#### 7.1 Login View
1. ASCII Plaza logo in PINK/CYAN at top
2. Centered form with username + masked password fields (Tab to switch focus, Enter to submit)
3. Error display below form; on success → transition to Now Playing

#### 7.2 Now Playing View
4. Left panel: album artwork (ratatui-image; fallback to Unicode block art via `image` crate pixel sampling)
5. Right panel: artist (PINK bold), title (CYAN bold), album (LAVENDER), duration gauge, listener count, reaction count
6. Controls hint bar at bottom of panel
7. `Space` → play/pause; `f` → toggle favorite; `r` → send reaction; `+/-` → volume

#### 7.3 History View
8. Scrollable list: `[timestamp]  Artist — Title  (duration)`
9. `j/k` scroll, `PgDn/PgUp` page jump; auto-load next page at bottom
10. `Enter` → song detail popup; `f` → add to favorites

#### 7.4 Favorites View
11. Same list style as history
12. `d`/`Delete` → confirm dialog → `remove_favorite()` → refresh
13. Pagination with auto-load

#### 7.5 Charts View
14. Sub-tabs: `[Overtime] [Weekly] [Monthly]` with `h/l` or arrow keys
15. Ranked list: `#1  Artist — Title   ♥ 4,521`
16. Pagination; `Enter` → song detail popup

#### 7.6 News View
17. List of items: timestamp + author (DIM), body text word-wrapped
18. `j/k` scroll, pagination

#### 7.7 Profile View
19. Username, email, member since; stats: reactions sent, favorites saved
20. `L` → logout confirm dialog → delete token → back to Login

#### 7.8 Help Overlay
21. `?` toggles centered overlay with keybindings table; any key dismisses

#### 7.9 Shared Widgets (`tui/widgets.rs`)
22. `vaporwave_block(title)` — DOUBLE border, PINK title, CYAN border color
23. `notification_bar(msg, style)` — status bar notification
24. `paginated_list(items, selected)` — styled list with scroll position indicator
25. `song_detail_popup(song)` — centered floating popup

---

### Phase 8 — Testing

1. `tests/api_models_test.rs`: fixture JSON → assert deserialization of all structs
2. `tests/hls_test.rs`: parse sample `.m3u8` strings; mock HTTP segment fetching with wiremock; test `select_stream` and `resolve_segment_url`
3. `tests/config_test.rs`: round-trip config to tempfile; test default creation
4. `tests/auth_test.rs` (wiremock): mock login endpoint → assert token returned; mock 401 → assert error type

---

### Phase 9 — Polish & Hardening

1. `tokio::signal::ctrl_c()` handler → stop player, restore terminal cleanly
2. Scrolling marquee in header for long song titles
3. `● LIVE` / `✗ DISCONNECTED` indicator in status bar
4. Volume OSD overlay (auto-dismiss after 1s)
5. Stream fallback: HLS fail 3× → switch to `/ogg` + notify user
6. Timed notification system (3s auto-dismiss): favorited, reaction sent, errors
7. In-memory artwork cache (keyed by URL) to avoid redundant fetches
8. `--reset-auth` CLI flag clears keyring token
9. README.md with install instructions, feature list, keybindings table, screenshot

---

## Keyboard Shortcuts Summary

| Key | Action |
|---|---|
| `1-6` | Switch view (Now Playing → Profile) |
| `j / k` | Scroll down / up |
| `J / K` | Scroll 5 lines down / up |
| `g / G` | Jump to top / bottom |
| `Enter` | Select / confirm |
| `Space` | Play / pause radio |
| `+ / -` | Volume up / down (5%) |
| `f` | Toggle favorite (current or selected song) |
| `r` | Send reaction to current song |
| `h / l` | Previous / next tab (Charts) |
| `d` | Remove favorite (Favorites view) |
| `L` | Logout (Profile view) |
| `?` | Toggle help overlay |
| `q` | Quit |

---

## Verification Checklist

After implementation, validate:
- [ ] `cargo build --release` succeeds with no warnings
- [ ] `cargo test` all pass
- [ ] `cargo clippy -- -D warnings` clean
- [ ] App launches, shows Login screen
- [ ] Login with valid credentials transitions to Now Playing
- [ ] Audio plays (verify with system audio output)
- [ ] Socket.io `status` event updates song info in real time
- [ ] History, Favorites, Charts, News all load and paginate correctly
- [ ] Favorite toggle (add and remove) persists
- [ ] Reaction sends and count updates
- [ ] Token persists across restarts (keyring)
- [ ] `--reset-auth` clears token
- [ ] Ctrl+C restores terminal cleanly
- [ ] Volume controls produce audible change
- [ ] Help overlay appears and dismisses
- [ ] Profile shows correct user data and logout works
- [ ] Works in terminals without image protocol support (fallback art)
