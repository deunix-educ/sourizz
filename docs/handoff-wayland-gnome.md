# Handoff: GNOME field test (second part)

Updated on 2026-10-30 on the field-test machine (`DNIX`, Debian 13, KDE Plasma 6.3.6, Intel UHD 630 + NVIDIA MX250). The user (Denis) writes French: answer in French, keep `README.md` and `README.fr.md` in sync.
Claude's per-machine memory does not travel, but the conversation of the previous session is kept per directory: resume it with `claude --continue` (or `claude --resume` to pick it) from `~/src/sourizz`.

## Where things stand

- **KDE Plasma Wayland on real hardware: done.** Overlay, ring, trail, click effects, click sounds, keystroke display, spotlight and annotation mode were confirmed by Denis in a real session. Both READMEs say so.
- **The "pointer freezes" bug is fixed** (commit `57bea85`): `Devices::rescan` reopened every uninteresting `/dev/input/event*` node every 3 s, and an open costs 25-70 ms on this machine (about 650 ms per rescan). The lesson and the method are in `CLAUDE.md`; `examples/layer_probe.rs` is the bisection tool.
- **GNOME 48.7 is installed next to KDE** (`gnome-shell`, `gnome-session`, `gnome-session-xsession`, `gdm3`). SDDM stays the login manager. It lists four GNOME entries: `GNOME on Wayland` and `GNOME on Xorg` are unambiguous; the two plain `GNOME` entries (one from `wayland-sessions`, one from `xsessions`) look identical, so avoid them and pick one of the other two.
- Debian 13 still ships the X11 session for GNOME 48. The README sentence "log in with GNOME on Xorg" is therefore right for this release; it would only be wrong from GNOME 49 on (Ubuntu 25.10, Fedora 43).

## Things to know before testing

- `~/.config/autostart/sourizz.desktop` exists: **a daemon starts by itself at each login.** Run `pgrep -ax sourizz`, then `sourizz quit`, before every manual test, and do not run two daemons on one session.
- The global hotkeys (`Ctrl+Alt+Shift+M`, `S`, `K`, `A`, …) toggle effects in the live daemon. Denis tested them by hand, which changed `enabled`, `spotlight` and `keystrokes` behind our back more than once. If a status looks odd, ask before assuming a bug, and note the md5 of `~/.config/sourizz/config.toml` with each measurement.
- Normal state: `enabled=on ring=on trail=on clicks=on sound=on shake=on`, `spotlight=off keystrokes=off focus=off`.
- Do not suggest adding the user to the `input` group again: it is already done and `sourizz doctor` reports it.

## Test plan for the GNOME sessions

Collect first, in each session, and paste the output back:
`echo $XDG_SESSION_TYPE $XDG_CURRENT_DESKTOP; gnome-shell --version; sourizz doctor; wayland-info | grep -i layer`

1. **GNOME (Wayland session). DONE on 2026-10-30.** The Wayland backend refuses to start with a clear message (exit code 1; the autostart log shows the same). With `sourizz --backend x11` (XWayland) the daemon starts, the ring follows the pointer inside an `xterm` and stays frozen over native Wayland windows, so XWayland is not a workaround. Both READMEs now say so. Not checked: whether the overlay itself is drawn above native Wayland windows.
2. **GNOME on Xorg.** Everything should work through the X11 backend: ring, trail, clicks, sound, keystrokes, spotlight, annotation, global hotkeys, settings window. Check the tray: the README says GNOME needs the *AppIndicator* extension; verify that claim (Debian package `gnome-shell-extension-appindicator`).
3. If anything differs from the README, fix both READMEs.

## Open decision: GNOME Wayland support

Not started, and not to be started without Denis agreeing on a design. Options to discuss once step 1 is done:

- A GNOME Shell extension (GJS) that draws or feeds the overlay, reads `global.get_pointer()` and registers keybindings, talking to the daemon over its Unix socket (`src/ipc.rs`).
- Documenting GNOME Wayland as unsupported, with the X11 session as the only route (valid for GNOME 48; to be revisited for 49 and later).

Say plainly what each option costs.

## When done

Update "What was tested" in both READMEs and the verification lines in `CLAUDE.md`, run `cargo test`, `cargo clippy --all-targets` and `cargo fmt`, then commit and push. Remove this file once its content has been absorbed, and drop the "Pending" line at the top of `CLAUDE.md`.
