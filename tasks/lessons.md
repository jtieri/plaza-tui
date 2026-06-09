# Lessons Learned

## 2026-03-06 - Architecture

**Mistake**: Writing log output to stdout in a TUI application
**Pattern**: stdout is the TUI rendering surface — any writes corrupt the display
**Rule**: All logging must go to file (tracing-subscriber with file appender). Never use println! or stdout directly
**Applied**: main.rs tracing setup, all debug/info logging throughout the app

---

## 2026-03-06 - Architecture

**Mistake**: Module declarations in main.rs/lib.rs not matching directory structure
**Pattern**: Rust module system requires exact correspondence between `mod` declarations and filesystem paths
**Rule**: When adding a new module, verify the `mod` declaration matches the file/directory path exactly
**Applied**: All module declarations in main.rs, lib.rs, and submodule mod.rs files

---

## 2026-03-06 - Architecture

**Mistake**: Not restoring terminal state on panic or Ctrl+C
**Pattern**: Raw mode + alternate screen persist after crash, leaving terminal unusable
**Rule**: Always install a panic hook that calls restore_terminal(), and handle Ctrl+C via event system to trigger graceful shutdown
**Applied**: main.rs panic hook, tui/events.rs Ctrl+C detection, app.rs shutdown sequence

---

## 2026-03-06 - Architecture

**Mistake**: Blocking on a single event source in the TUI loop
**Pattern**: TUI apps need to multiplex keyboard input, timers, socket events, and audio errors concurrently
**Rule**: Use tokio::select! to merge all event sources into a single async stream
**Applied**: tui/events.rs EventHandler::next(), app.rs main run loop

---

## 2026-06-03 - Research rigor

**Mistake**: Asserted "there is no pure-Rust Opus decoder" from prior knowledge when asked how
to decode Plaza's new Opus stream, instead of verifying current crate ecosystem state.
**Pattern**: Library/ecosystem capabilities change; stale training knowledge stated as fact
erodes trust and can steer architecture wrong. (There ARE pure-Rust options: opus-rs,
unsafe-libopus.)
**Rule**: Before asserting what crates/tools do or don't exist, verify against crates.io/docs.rs/
GitHub (esp. for "is there a library for X" questions). Distinguish confirmed facts from
inference. The user explicitly values deep research over confident guessing.
**Applied**: Re-ran the Opus/AAC/HLS decoder research via web + crates.io before recommending.

---

## 2026-06-03 - Debugging

**Mistake**: Initially suspected the user's code refactor caused audio to break.
**Pattern**: The real root cause was upstream (Plaza switched /ogg from Vorbis to Opus); the
runtime log (~/.local/share/plaza-tui/plaza-tui.log) showed it immediately ("selected opus
mapper" + "unsupported codec", 273 times today vs 580 vorbis before Apr 27).
**Rule**: For a behavior regression, read the app's own logs FIRST — they're ground truth and
often point straight at the cause before reading any source.
**Applied**: Diagnosed the Opus switch from the log; confirmed by probing live endpoints w/ ffprobe.
