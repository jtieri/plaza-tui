# Plaza TUI

A terminal UI client for [Nightwave Plaza](https://plaza.one/) — the vaporwave internet radio station. Listen to live radio, browse song history, manage favorites, react to tracks, and check the charts — all from your terminal with a full vaporwave aesthetic.

```
╔══════════════════════════════════════════════════════╗
║          ♫ NIGHTWAVE PLAZA                           ║
║          Macintosh Plus — リサフランク420 / 現代のコンピュー   ║
╚══════════════════════════════════════════════════════╝
┌────────────────┐ ╔══════════════════════════════════╗
│ ▶ Now Playing  │ ║ Now Playing                      ║
│   History      │ ║                                  ║
│   Favorites    │ ║ Macintosh Plus                   ║
│   Charts       │ ║ リサフランク420 / 現代のコンピュー   ║
│   News         │ ║ Floral Shoppe                    ║
│   Profile      │ ║ ▓▓▓▓▓▓░░░░░░░░ 2:31 / 4:12      ║
└────────────────┘ ║ ◉ 1,337 listening                ║
                   ║ ♥ 42 reactions                   ║
                   ╚══════════════════════════════════╝
● LIVE | ◉ 1337 listening | Vol: 80%      [?] Help [q] Quit
```

## Features

- **Live Audio Playback** — Pure Rust HLS stream via symphonia + rodio (no system audio deps)
- **Real-time Updates** — Socket.io connection for instant song change notifications
- **Song History** — Paginated scrollable history of recently played tracks
- **Favorites** — Add/remove favorites, synced with your Plaza account
- **Charts** — Overtime, weekly, and monthly song ratings
- **News** — Station news and announcements
- **Profile** — User stats (reactions sent, total favorites)
- **Vaporwave Aesthetic** — Hot pink, cyan, purple on dark navy
- **Secure Auth** — Tokens stored in your OS keyring (libsecret/Keychain/Windows Credential Manager)

## Installation

### Prerequisites

- Rust toolchain (1.70+): https://rustup.rs

### Build from source

```bash
git clone https://github.com/example/plaza-tui
cd plaza-tui
cargo build --release
# Binary at: target/release/plaza-tui
```

### Install

```bash
cargo install --path .
```

## Usage

```bash
# Launch the app
plaza-tui

# Use a specific stream quality
plaza-tui --stream-quality ogg

# Clear saved login token
plaza-tui --reset-auth

# Verbose logging
plaza-tui --log-level debug
```

### Stream quality options

| Flag | Stream |
|------|--------|
| `hls` (default) | HLS adaptive stream |
| `ogg` | Ogg Vorbis high quality |
| `ogg-low` | Ogg Vorbis low quality |
| `mp3` | MP3 high quality |
| `mp3-low` | MP3 low quality |

## Keyboard Shortcuts

| Key | Action |
|-----|--------|
| `1` | Now Playing |
| `2` | History |
| `3` | Favorites |
| `4` | Charts |
| `5` | News |
| `6` | Profile |
| `j` / `k` | Scroll down / up |
| `g` / `G` | Jump to top / bottom |
| `Enter` | Select / view song details |
| `Space` | Play / pause radio |
| `+` / `-` | Volume up / down (5%) |
| `f` | Toggle favorite (current or selected song) |
| `r` | Send reaction to current song |
| `h` / `l` | Previous / next chart tab |
| `d` | Remove favorite (Favorites view) |
| `L` | Logout (Profile view) |
| `?` | Toggle help overlay |
| `q` | Quit |

## Configuration

Config file: `~/.config/plaza-tui/config.toml`

```toml
stream_quality = "hls"   # hls | ogg | ogg-low | mp3 | mp3-low
volume = 0.8             # 0.0 to 1.0
image_protocol = ""      # "" = auto-detect | "kitty" | "sixel" | "iterm2"
```

## Logs

Application logs are written to:
- Linux: `~/.local/share/plaza-tui/plaza-tui.log`
- macOS: `~/Library/Application Support/plaza-tui/plaza-tui.log`

## Architecture

```
plaza-tui/
├── src/
│   ├── main.rs          # Entry point, CLI, logging
│   ├── app.rs           # AppState, main event loop, render dispatch
│   ├── config.rs        # Config struct, TOML persistence
│   ├── auth.rs          # Login/logout, OS keyring token storage
│   ├── error.rs         # PlazaError enum (thiserror)
│   ├── theme.rs         # Vaporwave color palette + ratatui styles
│   ├── socket.rs        # Socket.io client → broadcast channel
│   ├── api/
│   │   ├── models.rs    # Serde types (Song, User, StatusResource, ...)
│   │   ├── mod.rs       # ApiClient (reqwest wrapper)
│   │   └── client.rs    # All REST API methods
│   ├── audio/
│   │   ├── hls.rs       # HLS playlist fetching + segment streaming
│   │   └── player.rs    # Symphonia decode → rodio output
│   └── tui/
│       ├── events.rs    # AppEvent enum, EventHandler (keyboard+socket+audio)
│       ├── layout.rs    # Root layout (header/sidebar/content/statusbar)
│       ├── widgets.rs   # Reusable vaporwave-styled components
│       └── views/       # One file per screen
└── tests/               # Integration tests
```

## License

MIT
