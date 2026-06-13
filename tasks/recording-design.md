# Stream Recording → Local FLAC Library — Design

Capture full songs from the live stream as they play and build a local, properly
tagged FLAC library. This is the foundation for an on-demand local library and real
playlists (see the roadmap at the end).

## Non-negotiable correctness guarantee

**A saved file is always exactly one complete song — or it is not saved at all.**
Never a truncated song, never a premature cut, never two songs in one file, never a
corrupt container. We would rather drop a recording than persist a bad one.

This single rule drives every decision below.

### How we guarantee it

1. **Split only on exact, in-band boundaries** — never on a wall-clock guess or the
   (latency-skewed) socket metadata. The boundary must come from the audio stream
   itself, at the precise sample where one song ends and the next begins.
2. **Atomic writes** — encode to a temporary file (`.partial`) and `rename()` it into
   the library only after a *clean* finalize at a confirmed boundary. A crash or
   stream drop leaves only a discarded `.partial`; the library never contains a
   half-written file.
3. **Discard incomplete songs** — if the stream drops mid-song (we never see the
   song's end boundary), that recording is discarded, not saved. An incomplete song
   is worse than no song. Only a song captured start-boundary → end-boundary is kept.
4. **One song per file, by construction** — because we cut exactly at boundaries and
   start a fresh encoder per song, a file physically cannot contain two songs.
5. **Verify on finalize** — after encoding, confirm the FLAC decodes and its duration
   is sane before promoting it. A file that fails verification is discarded.

## Per-stream support (gated on splitting correctness)

We support recording **only where we can guarantee exact boundaries**. Where we
can't, recording is refused with a clear message — never attempted with ragged cuts.

| Stream | Boundary source | Recording |
|---|---|---|
| Opus — `/ogg`, `/ogg_low` | Chained-Ogg logical-stream boundary (the decoder reset we already handle) — exact | **v1** |
| MP3 — `/mp3`, `/mp3_low` | Icecast inline metadata (`Icy-MetaData`, `StreamTitle` at `icy-metaint`) — exact | **v1.1** |
| HLS — `/hls` | No reliable per-song boundary in-band | **Unsupported** |

### Do we force `stream_quality = "ogg"`?

No — but recording is **gated** on the active stream being splittable. If a user
enables recording while on HLS (or MP3 before v1.1), we do **not** silently produce
bad files and we do **not** silently change their setting. We show a clear notice:
*"Recording needs the OGG stream — set `stream_quality = ogg` to record."* Correct
files or none; never bad ones.

## Boundary + metadata (Opus, v1)

- **Boundary:** symphonia surfaces each chained-Ogg song as a new logical bitstream
  (`ResetRequired`). All PCM decoded *before* the reset is song A in full; PCM after
  is song B. This is sample-exact and free — we already handle it in the decode loop.
- **Metadata for naming/tags — CONFIRMED in-band.** A 5-minute live capture
  (2026-06-11) showed each song is a separate Ogg logical stream carrying its own
  `OpusTags` with `artist`, `album`, and `title`. We read these directly (symphonia
  surfaces them as metadata revisions on each stream reset), so naming and tagging
  are exact and need no socket correlation. The socket is not in the recorder's path
  at all — verified boundaries (BOS/EOS per song) + verified in-band tags.

## Filesystem layout & naming

- **Root:** configurable; default `<audio-dir>/Plaza/` (XDG `MUSIC`, else data dir).
- **Library path:** `<root>/<Artist>/<Album>/<Artist> - <Title>.flac`.
  - Standard artist/album tree so any music player (Plex, Navidrome, Jellyfin, …)
    indexes it cleanly.
  - Missing album → `Unknown Album`; missing artist → `Unknown Artist`.
  - Every path component is sanitized: strip `/\:*?"<>|` and control chars, trim
    trailing dots/spaces, collapse whitespace, cap length, never empty.
- **Dedup:** if the target file already exists, skip (we already have that track).
  Configurable, so a user can choose to keep duplicates.
- **Cover art:** download `artwork_src`, embed it as a FLAC `PICTURE` block, and also
  drop `cover.jpg` in the album folder (the convention players auto-detect).
- **Tags (Vorbis comments):** `ARTIST`, `ALBUM`, `TITLE`, plus a `COMMENT`/`SOURCE`
  noting it came from Nightwave Plaza and the capture date.

> Implementation note: `flacenc` (pure Rust) encodes the audio; FLAC metadata blocks
> (`VORBIS_COMMENT`, `PICTURE`) are a simple, well-specified format we can write
> ourselves if the crate doesn't expose them. To verify during implementation.

## Recording modes

- **off** (default) — no recording.
- **cache** — a rolling cache of the last *N* complete songs in a cache dir; oldest
  evicted past *N*. For "I liked that, let me grab it" after the fact.
- **session** — every complete song goes straight to the permanent library.

Plus a **keep** action (key) that promotes the current/last cached song into the
permanent library so the rolling cache won't evict it.

## Config (`[recording]`)

```toml
[recording]
mode = "off"            # off | cache | session
root = ""               # "" = <audio-dir>/Plaza
cache_size = 20         # rolling-cache song count
embed_artwork = true
deduplicate = true
```

## What this unlocks (why it's the keystone feature)

Once a local library of full FLAC files exists, on-demand features that a *live*
radio can't otherwise offer become possible:

- **Local library browser** — a new view to browse/search what you've recorded.
- **On-demand local playback** — a local-file `PcmSource` (symphonia decodes FLAC)
  plays library tracks through the existing `Player`. The radio client gains an
  on-demand mode.
- **Real playlists** — create/save/play playlists from the library (the original
  "playlists" idea, now actually playable).
- **Auto-keep favorites** — when a track you've favorited comes on the radio, keep it
  to the library automatically.
- **"In your library" indicator** — show on Now Playing whether the current song is
  already saved locally.

## Roadmap (sequenced)

- **3a — Recorder v1 (Opus):** decode-tap recorder, exact Ogg-boundary splitting,
  atomic writes, FLAC encode + tags + embedded/sidecar art, cache/session modes,
  keep key, config, status indicator. Tests: boundary→file mapping, sanitizer,
  atomic-finalize/discard-on-incomplete, FLAC round-trip.
- **3b — MP3 recording:** add `Icy-MetaData` parsing to the MP3 source for exact
  boundaries + titles.
- **3c — Local library + on-demand playback:** library index, browser view, local
  FLAC `PcmSource`, play recorded tracks on demand.
- **3d — Local playlists:** build/save/play playlists from the library.
- **3e — Niceties:** auto-keep favorites, "in your library" indicator, library stats.

(HLS recording remains unsupported until/unless a reliable boundary signal exists.)
