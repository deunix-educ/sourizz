# Handoff: field-testing Wayland and GNOME on another machine

Written on the X11/KDE dev machine on 2026-10-30 so that a Claude Code session on a second machine can pick the work up.
The user (Denis) writes French: answer in French, keep `README.md` and `README.fr.md` in sync.
Claude's per-machine memory does **not** travel: everything needed is in this file and in `CLAUDE.md`.

## Setup on the other machine

- Repo: `https://github.com/deunix-educ/sourizz` (private, branch `main`). Log in first (`gh auth login`, or an SSH key).
- Commit identity (the repo-local config does not travel): `git config user.name "Denis Defolie"` and `git config user.email denis.defolie@pm.me`.
- Build and install: `./install.sh --yes` (installs Rust with rustup if missing; needs a C linker and curl/wget; no `-dev` packages). For development use `cargo build --release` and run `target/release/sourizz`.
- Do not start two daemons on one session: `sourizz quit` the installed one first, or give the test one its own `XDG_RUNTIME_DIR`.

## What is verified, and what is not

- **X11:** exercised live on KDE Plasma 6.3 (hotkeys, settings window, tray, CLI).
- **Wayland:** only against a nested KWin 6.3. The evdev path (real mouse and keyboard) is unit-tested only. Never run on Hyprland, sway, niri, GNOME, or real HiDPI.
- **GNOME:** Mutter has no `wlr-layer-shell`, so the Wayland backend refuses to start (`src/wayland.rs`, message "this compositor does not support wlr-layer-shell"). The README tells GNOME users to log in with "GNOME on Xorg". No GNOME Shell extension exists.

## Test plan

1. Collect the facts and paste them back: `echo $XDG_SESSION_TYPE $XDG_CURRENT_DESKTOP`, the compositor version (`gnome-shell --version`, `kwin_wayland --version`, `Hyprland --version`, …), `sourizz doctor`, and `wayland-info | grep -i layer` (package `wayland-utils`).
2. Run `sourizz -vv --no-tray 2>&1 | tee sourizz.log` and keep the log.
3. **GNOME Wayland (expected: refuses to start).** Confirm the error is clear. Then try `sourizz --backend x11` (XWayland) and record what happens: is the overlay visible above native Wayland windows, does the ring follow the pointer over them or only over X11 windows? The README claims the latter; that claim is unverified.
4. **Check the "GNOME on Xorg" advice.** GNOME 49 and later no longer ship the X11 session (Ubuntu 25.10 and Fedora 43 dropped it). If the machine's GNOME has no Xorg session, the README sentence in both languages is wrong for that release: tell Denis, then fix both READMEs.
5. **Any layer-shell compositor (KDE Wayland, Hyprland, sway, …).** Check: overlay visible and click-through; ring follows the pointer and which pointer source is used (KWin script, Hyprland IPC, or the drifting estimate); two monitors; scale above 1; annotation mode (`sourizz annotate`) grabs and releases cleanly; hotkeys bound in the desktop as `sourizz toggle spotlight` etc.
6. Clicks, click sounds, keystrokes and hold-to-draw need `/dev/input` access, i.e. `sudo usermod -aG input "$USER"` and a new login. That is a security trade-off (any program of the account can then read keystrokes): ask Denis before suggesting it.
7. No input devices or no second machine? `CLAUDE.md` ("Testing Wayland without a Wayland session") explains the nested KWin setup with `SOURIZZ_FAKE_POINTER`.

## Open decision: GNOME support

Not started, and not to be started without Denis agreeing on a design. The options to discuss:

- A GNOME Shell extension (GJS) that draws or feeds the overlay, reads `global.get_pointer()` and registers keybindings, talking to the daemon over its Unix socket (`src/ipc.rs`).
- Documenting GNOME as unsupported, with the X11 session as the only route (only if step 4 shows that route still exists).

Use the test results above to choose, and say plainly what each option costs.

## When done

Update "What was tested" in both READMEs and the verification lines in `CLAUDE.md`, run `cargo test`, `cargo clippy --all-targets` and `cargo fmt`, then commit and push. Remove this file once its content has been absorbed.
