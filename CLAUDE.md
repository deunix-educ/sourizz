# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

> **Pending:** the GNOME part of the field test is still to do (KDE Wayland is done). If that is why you are here, read `docs/handoff-wayland-gnome.md` first.

## What this is

Sourizz is a Linux cursor highlighter (spotlight, ring, trails, click effects, keystroke display, on-screen drawing) for X11 and Wayland, written in Rust. It is an independent re-implementation of the idea behind the macOS app Mouzz. READMEs exist in English (`README.md`) and French (`README.fr.md`); keep them in sync. Licensed `AGPL-3.0-or-later` (`LICENSE`): new dependencies must be licence-compatible (MIT/Apache-2.0/BSD-style are fine).

## Commands

```sh
cargo build --release                    # daemon + settings window (first build is slow: egui)
cargo build --release --no-default-features   # daemon only
cargo test                               # 57 unit/property tests, ~3 s
cargo test --lib engine::tests::shake    # one test (name filter)
cargo clippy --all-targets && cargo fmt  # both must stay clean (rustfmt.toml: max_width 140)
cargo run --release -- demo out/         # render every effect to PNG, no display needed
cargo run -- -vv --no-tray               # run the daemon verbosely (-vv = debug for the sourizz crate only)
./install.sh [--uninstall] [--yes]       # user install into ~/.local; offers to install Rust via rustup if cargo is missing
```

`examples/x11_probe.rs` is an end-to-end check against a running X11 daemon (also `fake_keys`, `mover` helpers). Run the daemon with an isolated config: `XDG_CONFIG_HOME=<tmp> sourizz --no-tray`, with `[ring]\npulse_speed = 0.0` in `<tmp>/sourizz/config.toml` (the probe assumes the default 56 px ring, and the pulse changes the pixel counts of its ghost-pixel check); use absolute paths and check the daemon is up before starting the probe. The probe only touches its own test window, but a human moving the real mouse during the run makes it fail.

## Architecture

One drawing engine, two display backends, shared daemon core.

- `engine.rs` — **all** effect state and painting, no I/O. Input: `handle(now, InputEvent)`; output: `begin_frame(now) -> FramePlan{damage: Vec<IRect>, next_in}` then `paint(pixmap, region, now)`. Time is a caller-supplied `f64`, so it is fully testable. Sub-modules: `annot.rs` (strokes, toolbar, annotation-mode input), `keyviz.rs` (keystroke chips), `fx.rs` (tiny-skia primitives), `text.rs` (fontdue, font found via `fc-match`).
- `canvas.rs` — `Layer`: one per monitor, converts RGBA→premultiplied BGRA for damaged rects only. X11 uses a *streaming* layer (server keeps the pixels); Wayland a *retained* one (full buffer each commit).
- `x11.rs` — override-redirect ARGB32 window per monitor, empty XShape input region (click-through), XInput2 *raw* events for global input, `XGrabKey` hotkeys, real pointer+keyboard grab in annotation mode.
- `wayland.rs` — wlr-layer-shell surface per output, `wl_shm`, frame-callback pacing. Pointer position comes from `wlsrc.rs` (Hyprland IPC / dead-reckoning estimate) or `kwin.rs` (KWin script → D-Bus service `org.sourizz.Daemon`); clicks/keys from `evdev_in.rs`; layouts via `xkb.rs` (libxkbcommon through dlopen).
- `core.rs` — owns `Engine` + live `Config`; applies `Command`s (from tray `tray.rs`, CLI socket `ipc.rs`, hotkeys), hot-reloads the config file, plays sounds (`audio.rs`), keeps the tray in sync. `control.rs` has `Command`, `Hub` (channel + wake pipe) and `set_option`.
- `src/bin/settings.rs` — egui settings window; edits `config.toml`, the daemon reloads it. Gated by the default `gui` feature.

Adding a config option touches: `config.rs` (struct, `Default`, `sanitize`), `i18n.rs` (strings; enum values need an `opt.<id>` entry — a test enforces it), `settings.rs`, and both READMEs if user-visible.

## Gotchas learned the hard way

- **No `-dev` packages on the dev machine.** Keep the build pure Rust: `smithay-client-toolkit` has `default-features = false` (its `xkbcommon` feature needs `xkbcommon.pc`), keyboard handling is done by hand with `xkbcommon-dl`. `zbus` is pinned to 5 to match `ksni`.
- **x11rb queues events when you read a reply.** Always `poll_for_event()` once more right before sleeping in `poll(2)`, or events sit unseen for up to the idle timeout.
- **Damage model:** `Engine::bounds` returns `[IRect; 3]` slots; each slot is unioned with *its own* previous value so a moving edge (spotlight bar) covers the gap in between. Any change that alters pixels outside the reported bounds (monitor switch, config change) must force a full redraw. `canvas.rs` has a property test that incremental painting equals a full repaint — keep it green.
- After any command or config reload `Core::pump()` returns true; backends must repaint on it (forgetting this left changes invisible until the mouse moved).
- tiny-skia: `fill_rect` with a transform or anti-aliasing falls off its fast path (use `fx::fill_rect_fast`); `Rect::from_ltrb` accepts zero width and then fills a 1 px sliver; anti-aliasing is not perfectly translation-invariant (hence the tolerance in the canvas property test).
- evdev codes + 8 = xkb/X keycodes. The `wl_keyboard` keymap fd shares its file offset with the compositor: read it with `read_exact_at(.., 0)`.
- **egui ids must be unique on screen.** Two `ComboBox`es sharing an id (e.g. both labelled "Style") act as one control and their popups close each other, so the lists could not be opened. `combo()` in `settings.rs` takes an explicit id — never derive it from the label. `examples/click_at.rs` drives the real settings window through XTEST for such checks (it refuses to click unless the pointer is over our own window).
- **Raising an already-open settings window does not work under KWin.** Focus-stealing prevention turns another process's activation request (`ViewportCommand::Focus` after a `SIGUSR1`) into a blinking taskbar entry (`_NET_WM_STATE_DEMANDS_ATTENTION`), so a window buried under others stays buried. `Core::open_settings` therefore kills the old child and spawns a new one, which always maps on top. It is safe because the settings save as they change.
- Process name `sourizz-settings` is truncated to 15 chars by the kernel: use `pgrep -x sourizz-setting`, and never `pgrep -f` from a shell whose own command line contains the pattern.
- **Never reopen `/dev/input/event*` nodes on a timer.** On the field-test laptop (Intel + NVIDIA, KDE Plasma 6.3.6 Wayland) opening and closing one node costs 25-70 ms, so the old `Devices::rescan` (every 3 s, reopening every node it did not keep: power button, audio jacks, video bus…) blocked the daemon for ~650 ms and froze the pointer for that long, every 3 s, as soon as anything was drawn. `Devices` now remembers rejected nodes (`skipped`) and only opens new ones. How it was found, worth reusing: `examples/layer_probe.rs` is a bare layer-shell overlay with knobs (`--size`, `--fps`, `--damage`, `--follow` for the KWin pointer script, `--seat`, `--evdev`, `--no-rescan`); adding Sourizz's pieces to it one at a time showed which one brought the stall back. Two traps: the "callback delay" I first measured was the time until *our* loop handled the callback, so it showed our own stall as if it were the compositor's; and a loop timer that omits a section (here the rescan, after `poll`) cannot see a stall in it. Record the config md5 with each run, since a changed config once made a test look clean.

## Testing Wayland without a Wayland session

Run a private compositor with its own bus and a *short* runtime dir (socket paths are limited to 108 bytes):
`XDG_RUNTIME_DIR=/tmp/x dbus-run-session -- kwin_wayland --virtual --socket sz --no-lockscreen --no-global-shortcuts --width 1280 --height 720`, then start `sourizz --backend wayland --no-tray` with `WAYLAND_DISPLAY=sz`, the same `XDG_RUNTIME_DIR`, the nested `DBUS_SESSION_BUS_ADDRESS` and (no input devices there) `SOURIZZ_FAKE_POINTER=640,360`. `spectacle -b -n -f -o file.png` in that environment screenshots the nested compositor. Real input devices (the evdev path) cannot be tested this way.
