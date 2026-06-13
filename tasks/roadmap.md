# Plaza TUI — Development Roadmap

A living roadmap. Done items stay for context; upcoming items are ordered roughly by
priority and dependency.

## Done

- **Audio fixes** — restored playback (Plaza moved `/ogg` to Opus), killed the
  reconnect storm, fixed `_low` URLs, hardened key input.
- **Full codec parity** — MP3, Opus (libopus), and HLS/AAC (MPEG-TS demux), behind a
  codec-agnostic `PcmSource` layer; HLS drop-out + drift fix (bounded-latency,
  background-fetch design).
- **Productionization** — Cargo workspace (`plaza-api`, `plaza-audio`, `plaza-tui`),
  per-crate `thiserror`, full public-API docs, clippy `-D warnings` clean, CI across
  Linux/macOS/Windows, `justfile`, README + dual license.
- **Sleep timer** — `t` cycles off / 15 / 30 / 60 min; pauses on elapse.
- **Favorites export** — `e` triggers Plaza's CSV export and surfaces the link.

## In progress / next

### Recording → local FLAC library (the keystone) — see `recording-design.md`
Capture full songs from the live stream into a tagged FLAC library. Correctness is
non-negotiable: exact in-band splits, atomic writes, discard incomplete songs.
- **3a DONE** Recorder v1 (Opus): exact Ogg-boundary splitting, FLAC encode (with a
  STREAMINFO fix-up so symphonia can read it back), in-band OpusTags, embedded cover
  art, off/cache/session modes, `R` cycle + `s` keep keys, `[recording]` config,
  status indicator. Validated end-to-end against the live stream (records a real song
  to a valid, decodable FLAC). Recording is gated to Opus.
- **3b** MP3 recording via `Icy-MetaData`.
- **3c** Local library + on-demand playback (local FLAC `PcmSource`, browser view).
- **3d** Local playlists (build / save / play from the library).
- **3e** Auto-keep favorites; "in your library" indicator; library stats.

### Remaining API / UX parity (independent, pick up between recorder phases)
- News "latest/unread" badge.
- Account: register, profile edit, change password, delete account.
- UX polish: volume OSD overlay, scrolling marquee for long header titles.

## Notes / constraints

- Plaza is **live radio** — no on-demand catalog and no full-track download API. The
  recorder is what makes an on-demand local library (and real playlists) possible.
- HLS has no reliable in-band song boundary, so recording is Opus/MP3 only.
- No song-search / artist-page / request APIs exist server-side; those aren't buildable.
