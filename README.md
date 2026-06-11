# Plaza TUI

A terminal client for [Nightwave Plaza](https://plaza.one/), the vaporwave internet radio
station. Listen to the live stream, browse history, manage favorites, react to tracks, and
read the charts — all from your terminal, in full vaporwave color.

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

- **Every Plaza stream format** — MP3, Opus, and adaptive HLS/AAC, all decoded in-process.
- **Real-time updates** — a Socket.IO connection reflects song changes, listener counts, and
  reactions live, reconnecting automatically.
- **History, favorites, charts, news** — browse and paginate everything Plaza exposes.
- **Account features** — log in to favorite tracks, send reactions, and view your stats.
- **Album art in the terminal** — sixel, kitty, and iTerm2 image protocols, with a graceful
  fallback.
- **Secure auth** — tokens are kept in the OS keyring, with an encrypted-at-rest file fallback.

## Install

### Prerequisites

- A Rust toolchain, 1.85 or newer ([rustup.rs](https://rustup.rs)).
- A C compiler and **CMake** — the Opus decoder builds libopus from source, so released
  binaries need no system audio library at runtime.
- **Linux only:** ALSA development headers (`libasound2-dev` on Debian/Ubuntu,
  `alsa-lib` on Arch) for audio output.

### From source

```bash
git clone https://github.com/jtieri/plaza-tui
cd plaza-tui
cargo install --path crates/plaza-tui
# or: cargo build --release   →   target/release/plaza-tui
```

## Usage

```bash
plaza-tui                          # launch
plaza-tui --stream-quality ogg     # pick a stream (see below)
plaza-tui --reset-auth             # forget the saved login token
plaza-tui --log-level debug        # more verbose file logging
```

### Stream qualities

Pass `--stream-quality <name>`, or set it in the config file. MP3 is the default — it plays
everywhere with no extra setup.

| Name | Codec | Bitrate |
|------|-------|---------|
| `mp3` (default) | MP3 | 128 kbps |
| `mp3-low` | MP3 | 96 kbps |
| `ogg` | Opus | 64 kbps |
| `ogg-low` | Opus | 96 kbps |
| `hls` | AAC (adaptive) | up to ~141 kbps |

> HLS buffers a few seconds for smooth playback, so the "Now Playing" panel can run slightly
> ahead of the audio; the lower-latency MP3 and Opus streams stay closely in sync.

## Keyboard shortcuts

| Key | Action |
|-----|--------|
| `1`–`6` | Switch view (Now Playing, History, Favorites, Charts, News, Profile) |
| `j` / `k` | Scroll down / up |
| `J` / `K` | Scroll five lines |
| `g` / `G` | Jump to top / bottom |
| `Enter` | View song details |
| `Space` | Play / pause |
| `+` / `-` | Volume up / down |
| `f` | Toggle favorite (current or selected song) |
| `r` | Send a reaction to the current song |
| `h` / `l` | Previous / next chart range |
| `d` | Remove favorite (Favorites view) |
| `p` | Play / stop a song's preview (in the detail popup) |
| `L` | Log out (Profile view) |
| `?` | Toggle the help overlay |
| `q` / `Ctrl-C` | Quit |

## Configuration

`~/.config/plaza-tui/config.toml` (or the platform equivalent), created on first run:

```toml
stream_quality = "mp3"   # mp3 | mp3-low | ogg | ogg-low | hls
volume = 0.8             # 0.0 to 1.0
image_protocol = ""      # "" = auto-detect | "kitty" | "sixel" | "iterm2"
```

Logs are written to `~/.local/share/plaza-tui/plaza-tui.log` (the platform data directory).

## Architecture

A Cargo workspace of two libraries and a binary, with dependencies pointing inward toward the
domain — the UI and frameworks stay at the edges:

```
crates/
  plaza-api/     REST client, Socket.IO feed, auth/keyring, response models
  plaza-audio/   the Player, a codec-agnostic PcmSource pipeline, and the
                 MP3 / Opus / HLS-AAC decoders (incl. a small MPEG-TS demuxer)
  plaza-tui/     the binary: config, application state, and the ratatui UI
```

`plaza-api` and `plaza-audio` have no dependency on each other or on the UI; the binary wires
them together. Each library exposes its own `thiserror` error type; the binary uses `anyhow`.

## Development

```bash
just ci        # format check + clippy (-D warnings) + tests + docs (the CI gate)
just test
just run -- --stream-quality ogg
just smoke     # live-network decode tests against radio.plaza.one (normally skipped)
```

[`just`](https://github.com/casey/just) is optional — each recipe is a thin wrapper over the
equivalent `cargo` command. CI runs the same gates on Linux, macOS, and Windows.

## License

Licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT) at your option.
