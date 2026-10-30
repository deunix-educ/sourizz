# Sourizz

**English** · [Français](README.fr.md)

A cursor highlighter for Linux. Put a **spotlight**, a **ring**, a **trail** or a **click ripple** on your pointer, show your **keystrokes**, and **draw on the screen** — for presentations, tutorials, screencasts, or just to stop losing your cursor on a big monitor. Works on **X11 and Wayland**.

Sourizz is an independent Linux re-implementation of the idea behind [Mouzz](https://mouzz.dev) for macOS. It shares no code or assets with it and is not affiliated with or endorsed by its author.

![Ring and spotlight](docs/img/effects.png)

## Features

| | |
|---|---|
| **Spotlight** | Dims everything except a soft-edged circle, horizontal bar or vertical bar around the pointer. Radius, opacity, softness and colour are adjustable. |
| **Ring cursor** | A ring that follows the pointer, with a gentle pulse. Classic or Neon style; size, thickness, colour, fill and pulse speed are adjustable. |
| **Trails** | Seven styles: Comet, Dots, Ribbon, Rainbow, Stars, Kawaii, Sparkles. |
| **Click effects** | Single ring, ripple wave, burst or pulse, with a different colour for left, middle and right button. Can also be triggered on demand by a key tap or a hotkey. |
| **Click sounds** | A built-in synthesised click, or your own WAV file. |
| **Shake to locate** | Shake the mouse and the ring swells so you can find the pointer. |
| **Draw on screen** | Hold a key and move the pointer to draw fading strokes. Or open the **annotation mode**: pen, highlighter, text, numbered callouts, eraser, undo. |
| **Keystroke visualiser** | Six styles (Classic, Glass, Keycaps, Neon, Gradient, Minimal). Typing is merged into a running text chip; shortcuts show as `Ctrl + Shift + T`. An "only shortcuts" option hides plain typing. |
| **Focus mode** | Dims everything except the active window (X11 only for now). |
| **Everything else** | System tray icon with toggles and style pickers, a settings window, global hotkeys, a command line for scripting, English and French UI. Settings apply live. |

![Trails](docs/img/trails.png)
![Click effects](docs/img/clicks.png)
![Keystrokes](docs/img/keystrokes.png)
![Annotation mode](docs/img/annotate.png)
![Focus mode](docs/img/focus.png)

### Compared with Mouzz

Sourizz covers Mouzz's free feature set (spotlight, ring, trails, click effects and sounds, shake-to-locate, drawing, global hotkeys, menu-bar/tray control) and, from its Pro tier, the keystroke visualiser, focus mode, and the highlighter, text and numbered-callout annotation tools.

Differences and gaps:

- **Hold-to-draw key.** Mouzz uses the `Fn` key, which Linux keyboards usually do not report at all (the firmware handles it). The default here is the right `Ctrl` key, configurable in the settings.
- **Not implemented:** the screen recorder, automatic captions, camera bubble, live zoom / auto-zoom on click and the loupe magnifier. They are separate products on Linux (OBS, `wf-recorder`, KDE's Zoom effect…).
- **Focus mode** needs the active window's geometry, which only X11 provides to every program.

## Install

You need a recent stable [Rust toolchain](https://rustup.rs) (developed with 1.98; the crate uses edition 2024). **No `-dev` packages are required**: the build is pure Rust and loads `libxkbcommon` at run time. The first build takes a few minutes because the settings window pulls in egui; `cargo build --release --no-default-features` builds the daemon alone. If Rust is missing, `./install.sh` offers to install it with rustup (it needs `curl` or `wget` and a C linker such as `build-essential`; add `--yes` to skip the question).

```sh
git clone <this repository> sourizz && cd sourizz
./install.sh                # installs into ~/.local (add --uninstall to remove)
sourizz doctor              # checks your system
sourizz                     # start the daemon (a tray icon appears)
sourizz autostart enable    # optional: launch at login
```

Run-time requirements:

- **X11:** a compositing window manager (KWin, Mutter, Xfwm4, Picom…) so the overlay can be transparent. The X server needs XInput2, RandR and Shape (all standard).
- **Wayland:** a compositor that implements `wlr-layer-shell` — see the [compatibility table](#x11-and-wayland).
- **Optional:** `pw-play`, `paplay` or `aplay` for click sounds; a system tray that understands the StatusNotifierItem standard (KDE does — tested; GNOME needs the *AppIndicator* extension; elsewhere it depends on your panel — without a tray everything still works through the command line, hotkeys and the settings window); OpenGL for the settings window.

## Using it

**Tray icon.** Left click toggles everything; right click opens the menu with every effect, style pickers, annotation mode, settings and quit.

**Settings window.** `sourizz settings` (or the tray menu). Changes are applied to the running daemon immediately and saved automatically.

![Settings window](docs/img/settings.png)

**Hotkeys** (X11; configurable). On Wayland, bind the [commands below](#command-line) in your desktop's shortcut settings instead — see [Wayland setup](#wayland-setup).

| Default | Action |
|---|---|
| `Ctrl+Alt+Shift+M` | Master switch |
| `Ctrl+Alt+Shift+S` | Spotlight |
| `Ctrl+Alt+Shift+R` | Ring |
| `Ctrl+Alt+Shift+T` | Trail |
| `Ctrl+Alt+Shift+C` | Click effects |
| `Ctrl+Alt+Shift+K` | Keystrokes |
| `Ctrl+Alt+Shift+F` | Focus mode |
| `Ctrl+Alt+Shift+A` | Annotation mode |
| `Ctrl+Alt+Shift+X` | Clear annotations |
| `Ctrl+Alt+Shift+Y` | Show the click effect at the pointer |

Annotation mode and the click preview switch on what they need (the master switch, and the click effects for the preview) when it is off.

**Drawing.** Hold the right `Ctrl` key and move the pointer: a stroke follows and fades away after you release. For more control open the **annotation mode**: a toolbar appears and the overlay captures the mouse and keyboard.

In annotation mode: `P` pen, `H` highlighter, `T` text, `N` numbered callout, `E` eraser, `1`–`6` colour, `+`/`-` width, `Ctrl+Z` or `Backspace` undo, `Delete` clear, `Esc` leave. While typing text, `Enter` validates.

### Command line

Everything the tray does is scriptable; this is also how you bind shortcuts on Wayland.

```sh
sourizz toggle spotlight        # all, spotlight, ring, trail, clicks, sound, shake,
sourizz on trail                #   annotate, keystrokes, focus
sourizz off keystrokes
sourizz set ring.size 90        # any option from the config file
sourizz set trail.style stars
sourizz annotate                # toggle annotation mode  (annotate clear: erase)
sourizz show-click              # play the click effect at the pointer
sourizz status                  # print the current state
sourizz settings | reload | quit
sourizz config                  # print the config file path (--defaults: the defaults)
sourizz doctor                  # diagnose your setup
```

### Configuration

`~/.config/sourizz/config.toml`; every key is optional. The file is watched, so edits apply instantly. `sourizz config --defaults` prints the complete list.

```toml
[ring]
style = "neon"          # classic | neon
size = 64.0
color = "#00E5FF"

[spotlight]
enabled = true
shape = "circle"        # circle | hbar | vbar
radius = 200.0
opacity = 0.7

[trail]
enabled = true
style = "kawaii"        # comet | dots | ribbon | rainbow | stars | kawaii | sparkles

[clicks]
style = "ripple_wave"   # single_ring | ripple_wave | burst | pulse
show_on_clicks = true
tap_key = "Control_L"   # also show the effect when this key is tapped (empty = off)

[annotate]
hold_key = "Control_R"  # hold + move to draw; empty = off

[hotkeys]
spotlight = "Ctrl+Alt+Shift+S"
```

## X11 and Wayland

Sourizz has one drawing engine and two display backends; it picks the right one automatically (`--backend x11|wayland` or `general.backend` to force one).

| Session | Overlay | Pointer position | Clicks and keys | Focus mode |
|---|---|---|---|---|
| **X11** (any compositing WM) | yes | exact | yes, via XInput2 | yes |
| **KDE Plasma, Wayland** | yes | exact, via a KWin script | needs the `input` group | – |
| **Hyprland** | yes | exact, via its IPC socket | needs the `input` group | – |
| sway, river, niri, labwc, COSMIC… | yes | *estimated* (see below) | needs the `input` group | – |
| **GNOME, Wayland** | **no** — Mutter has no layer-shell | – | – | – |

On GNOME, log in with a **"GNOME on Xorg"** session, or use another desktop. (A GNOME Shell extension could do it natively; that is not written yet.)

### Choosing the backend

By default Sourizz follows your session: it uses Wayland when `WAYLAND_DISPLAY` is set and `$XDG_SESSION_TYPE` is not `x11`, and X11 otherwise. `sourizz doctor` prints the session and the backend that will be used (`echo $XDG_SESSION_TYPE` also tells you which session you are in).

To force one, use `sourizz --backend x11` or `--backend wayland`, or set `backend = "x11"` (`"wayland"`, `"auto"`) under `[general]` in the config file — the settings window has it in the *General* tab. The command-line flag wins over the file. The backend is chosen at start-up, so **restart the daemon** after changing it. Forcing X11 inside a Wayland session is rarely useful: an X11 program there generally only sees the pointer while it is over other X11 windows.

The session itself is chosen at the login screen (for example "Plasma (X11)" or "Plasma (Wayland)").

**Why Wayland is harder.** By design a Wayland program cannot see the pointer or the keyboard while another window has them. Sourizz works around this without any privileged helper:

- the overlay is a click-through `wlr-layer-shell` surface (an empty input region lets every click pass);
- the **pointer position** comes from the compositor when it offers a way (KWin, Hyprland); otherwise it is *estimated* by adding up relative mouse motion, which drifts because the compositor applies pointer acceleration Sourizz cannot see. Treat it as approximate there;
- **clicks and keystrokes** are read from `/dev/input/event*`;
- the **keyboard layout** for key labels is read from `XKB_DEFAULT_LAYOUT`, `/etc/default/keyboard` or KDE's `kxkbrc`;
- **annotation mode** takes a real pointer and keyboard grab, so it is exact on every compositor.

### Wayland setup

1. Allow Sourizz to read input devices (only needed for click effects, click sounds, the keystroke visualiser, hold-to-draw and in-app hotkeys; the ring, spotlight and trail work without it on KDE and Hyprland):

   ```sh
   sudo usermod -aG input "$USER"      # then log out and back in
   ```

   > **Security note.** Members of `input` can read every keystroke on the machine, so any program you run could. Sourizz only turns events into on-screen effects and hotkey matches — it stores nothing, logs nothing and has no network code — but you are granting the permission to your whole user account. If you would rather not, skip this step: the overlay and everything driven by the pointer position still works on KDE and Hyprland, and annotation mode never needs it.

2. Bind shortcuts in your desktop, because Wayland has no global-hotkey API for plain programs (Sourizz also matches your configured hotkeys from the raw keyboard when it has access, but that cannot *consume* the key press):
   - **KDE:** System Settings → Keyboard → Shortcuts → Add Command → `sourizz toggle spotlight`
   - **Hyprland:** `bind = SUPER SHIFT, S, exec, sourizz toggle spotlight`
   - **sway:** `bindsym $mod+Shift+s exec sourizz toggle spotlight`

3. `sourizz doctor` tells you what is missing.

Known limitation: on HiDPI outputs with a scale above 1 the overlay is drawn at 1× and upscaled by the compositor, so it looks slightly soft (positions are correct).

Known limitation: on KDE Plasma Wayland (KWin 6.3.6, laptop with an Intel UHD 630 + NVIDIA MX250 hybrid GPU) the pointer freezes for about half a second every two to three seconds while a **trail** is drawn. The ring alone does not trigger it, and neither does Sourizz with all effects off. Sourizz itself is not blocked during these stalls and the area it redraws is small, so the cause lies in how KWin handles the continuous updates on this hardware; it is not understood yet. On such a setup, turn the trail off (`sourizz set trail.enabled false`) or use the X11 session. The ring can also lag slightly behind a fast-moving pointer on Wayland, because its position takes a longer path (KWin script, then D-Bus) than the raw XInput2 events used on X11.

## What was tested

Being honest about verification:

- **Automated:** 57 unit tests (config, engine, damage tracking, rendering invariants, key handling with real AZERTY/QWERTY layouts, evdev translation, IPC parsing). Notably, a property test checks that incremental repainting is pixel-identical to a full repaint for every spotlight shape, trail style and click effect.
- **X11:** exercised live on Debian 13, KDE Plasma 6.3, two monitors — overlay windows, click-through, ring/click/trail drawing, no ghost pixels after fast motion, global hotkey, CLI, live config reload, tray menu over D-Bus, settings window.
- **Wayland:** exercised against a nested **KWin 6.3** compositor — layer-shell overlay, ring/spotlight/annotation toolbar rendering, annotation mode capture, the KWin pointer script. Also run in a real KDE Plasma 6.3.6 Wayland session (see the trail limitation above): the ring follows the pointer through the KWin script and the real mouse, touchpad and keyboard devices are opened. **Not verified** on a real session: click effects and sounds, keystroke display and annotation mode. **Not tested** on real HiDPI, GNOME, or Hyprland, sway, niri or other compositors.

Treat Wayland support as a solid first version that needs more field testing; bug reports with `sourizz -vv` output are welcome.

## Privacy

Everything runs locally. Sourizz makes no network connections and writes nothing but its config file (and a few runtime files: the control socket, a small click WAV and the KWin script under `$XDG_RUNTIME_DIR`).

The keystroke visualiser shows what you type; it is **off by default**. When screen-sharing, prefer "only shortcuts" so passwords are never displayed.

## Development

```sh
cargo test                                    # unit + property tests
cargo run --release -- demo out/              # render every effect to PNGs, no display needed
cargo run --example x11_probe -- out/         # end-to-end X11 check against a running daemon
RUST_LOG=sourizz=debug cargo run -- -vv       # verbose daemon
```

Layout: `engine.rs` (state + drawing, no I/O), `canvas.rs` (damage-limited per-monitor buffers), `x11.rs` / `wayland.rs` (backends), `core.rs` (config, commands, tray, sound), `evdev_in.rs` / `wlsrc.rs` / `kwin.rs` / `xkb.rs` (Wayland input), `src/bin/settings.rs` (egui settings window). See `CLAUDE.md` for the gotchas.

To test Wayland without a Wayland session you can run a private compositor: `dbus-run-session -- kwin_wayland --virtual --socket sz --width 1280 --height 720` with its own `XDG_RUNTIME_DIR`, then start Sourizz with `WAYLAND_DISPLAY=sz` and `SOURIZZ_FAKE_POINTER=640,360`.

## License

Sourizz is free software, released under the **GNU Affero General Public License, version 3 or (at your option) any later version** (`AGPL-3.0-or-later`). See [LICENSE](LICENSE).

In short: you may use, study, modify and share it, including commercially, as long as anything you distribute — or offer to users over a network — built on it stays under the same licence with its source available.

Mouzz is a product of its respective owner; the name is used here only to describe what Sourizz is inspired by.
