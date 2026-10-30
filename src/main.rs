use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use sourizz::config::{BackendKind, Config};
use sourizz::control::Hub;
use sourizz::core::Core;
use sourizz::{demo, ipc, wayland, x11};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

#[derive(Parser)]
#[command(
    name = "sourizz",
    version,
    about = "Cursor highlighter for Linux: spotlight, ring, trails, click effects and annotations (X11 and Wayland).\nRun without a subcommand to start the daemon; the other subcommands control a running daemon."
)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
    /// Display backend (overrides the config file).
    #[arg(long, value_enum, global = false)]
    backend: Option<BackendArg>,
    /// Do not create a system tray icon.
    #[arg(long)]
    no_tray: bool,
    /// More log output (-v info, -vv debug).
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

#[derive(Clone, Copy, ValueEnum)]
enum BackendArg {
    Auto,
    X11,
    Wayland,
}

#[derive(Subcommand)]
enum Cmd {
    /// Start the daemon (same as running with no subcommand).
    Run,
    /// Flip an effect: all, spotlight, ring, trail, clicks, sound, shake, annotate, keystrokes, focus.
    Toggle { effect: String },
    /// Turn an effect on.
    On { effect: String },
    /// Turn an effect off.
    Off { effect: String },
    /// Set any option, e.g. `sourizz set ring.size 80` or `sourizz set trail.style stars`.
    Set { key: String, value: Vec<String> },
    /// Play the click effect at the pointer.
    ShowClick,
    /// Annotation mode: `toggle` (default) or `clear`.
    Annotate { action: Option<String> },
    /// Re-read the config file.
    Reload,
    /// Print the daemon's state.
    Status,
    /// Open the settings window.
    Settings,
    /// Stop the daemon.
    Quit,
    /// Print the config file path, or its defaults with `--defaults`.
    Config {
        #[arg(long)]
        defaults: bool,
    },
    /// Start or stop launching Sourizz at login.
    Autostart { action: AutostartAction },
    /// Check this system for everything Sourizz needs.
    Doctor,
    /// Render every effect to PNG files in a directory (no display needed).
    #[command(hide = true)]
    Demo { dir: PathBuf },
}

#[derive(Clone, Copy, ValueEnum)]
enum AutostartAction {
    Enable,
    Disable,
}

fn main() {
    if let Err(e) = real_main() {
        eprintln!("sourizz: {e:#}");
        std::process::exit(1);
    }
}

fn real_main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        None | Some(Cmd::Run) => run_daemon(&cli),
        Some(Cmd::Toggle { effect }) => remote(&format!("toggle {effect}")),
        Some(Cmd::On { effect }) => remote(&format!("on {effect}")),
        Some(Cmd::Off { effect }) => remote(&format!("off {effect}")),
        Some(Cmd::Set { key, value }) => remote(&format!("set {key} {}", value.join(" "))),
        Some(Cmd::ShowClick) => remote("show-click"),
        Some(Cmd::Annotate { action }) => remote(&format!("annotate {}", action.unwrap_or_default())),
        Some(Cmd::Reload) => remote("reload"),
        Some(Cmd::Status) => remote("status"),
        Some(Cmd::Quit) => remote("quit"),
        Some(Cmd::Settings) => settings(),
        Some(Cmd::Config { defaults }) => {
            if defaults {
                print!("{}", toml::to_string_pretty(&Config::default())?);
            } else {
                println!("{}", Config::path().display());
            }
            Ok(())
        }
        Some(Cmd::Autostart { action }) => autostart(action),
        Some(Cmd::Doctor) => doctor(),
        Some(Cmd::Demo { dir }) => demo::run(&dir),
    }
}

fn remote(line: &str) -> Result<()> {
    match ipc::send(line) {
        Ok(reply) => {
            if let Some(err) = reply.strip_prefix("error:") {
                bail!("{}", err.trim());
            }
            if reply != "ok" {
                println!("{reply}");
            }
            Ok(())
        }
        Err(e) if matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused) => {
            bail!("the daemon is not running — start it with `sourizz`")
        }
        Err(e) => Err(e).context("talking to the daemon"),
    }
}

fn settings() -> Result<()> {
    let sibling = std::env::current_exe()?.with_file_name("sourizz-settings");
    let program = if sibling.is_file() { sibling } else { PathBuf::from("sourizz-settings") };
    let err = std::os::unix::process::CommandExt::exec(std::process::Command::new(&program).arg0("sourizz-settings"));
    bail!("cannot start {}: {err}", program.display())
}

use std::os::unix::process::CommandExt as _;

fn session_is_wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some() && std::env::var("XDG_SESSION_TYPE").map(|s| s != "x11").unwrap_or(true)
}

fn pick_backend(cli: &Cli, cfg: &Config) -> BackendKind {
    let want = match cli.backend {
        Some(BackendArg::X11) => BackendKind::X11,
        Some(BackendArg::Wayland) => BackendKind::Wayland,
        Some(BackendArg::Auto) => BackendKind::Auto,
        None => cfg.general.backend,
    };
    match want {
        BackendKind::Auto if session_is_wayland() => BackendKind::Wayland,
        BackendKind::Auto => BackendKind::X11,
        other => other,
    }
}

fn run_daemon(cli: &Cli) -> Result<()> {
    let level = match cli.verbose {
        0 => "warn",
        1 => "info",
        _ => "debug",
    };
    // Only our own crate follows -v; zbus & co. stay quiet unless RUST_LOG says otherwise.
    let mut b = env_logger::Builder::new();
    b.filter_level(log::LevelFilter::Warn).filter_module("sourizz", level.parse().unwrap_or(log::LevelFilter::Warn)).format_timestamp(None);
    if let Ok(spec) = std::env::var("RUST_LOG") {
        b.parse_filters(&spec);
    }
    b.init();

    let cfg = Config::load();
    if ipc::is_running() {
        bail!("sourizz is already running (use `sourizz quit` to stop it)");
    }
    let (hub, rx, wake) = Hub::new()?;
    ipc::serve(hub.clone()).context("starting the control socket")?;

    let term = Arc::new(AtomicBool::new(false));
    for sig in [signal_hook::consts::SIGINT, signal_hook::consts::SIGTERM, signal_hook::consts::SIGHUP] {
        signal_hook::flag::register(sig, term.clone())?;
        let w = rustix::io::fcntl_dupfd_cloexec(&*hub.wake_fd(), 0)?;
        signal_hook::low_level::pipe::register(sig, w)?;
    }

    let backend = pick_backend(cli, &cfg);
    let mut core = Core::new(cfg, hub, rx);
    core.set_term_flag(term);
    if !cli.no_tray {
        core.start_tray();
    }
    log::info!("starting with the {} backend", backend.id());
    let result = match backend {
        BackendKind::Wayland => wayland::run(core, &wake),
        _ => x11::run(&mut core, &wake),
    };
    ipc::cleanup();
    result
}

fn autostart_path() -> PathBuf {
    Config::dir().parent().map(|p| p.join("autostart")).unwrap_or_else(|| PathBuf::from(".config/autostart")).join("sourizz.desktop")
}

fn autostart(action: AutostartAction) -> Result<()> {
    let path = autostart_path();
    match action {
        AutostartAction::Enable => {
            let exe = std::env::current_exe()?;
            std::fs::create_dir_all(path.parent().unwrap())?;
            std::fs::write(
                &path,
                format!(
                    "[Desktop Entry]\nType=Application\nName=Sourizz\nComment=Cursor highlighter\nExec={}\nIcon=input-mouse\nTerminal=false\nX-GNOME-Autostart-enabled=true\n",
                    exe.display()
                ),
            )?;
            println!("autostart enabled: {}", path.display());
        }
        AutostartAction::Disable => match std::fs::remove_file(&path) {
            Ok(()) => println!("autostart disabled"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => println!("autostart was not enabled"),
            Err(e) => return Err(e.into()),
        },
    }
    Ok(())
}

fn doctor() -> Result<()> {
    let ok = |b: bool| if b { "ok     " } else { "MISSING" };
    let env = |k: &str| std::env::var(k).unwrap_or_else(|_| "-".into());
    println!("session      : {} ({})", env("XDG_SESSION_TYPE"), env("XDG_CURRENT_DESKTOP"));
    println!("DISPLAY      : {}   WAYLAND_DISPLAY: {}", env("DISPLAY"), env("WAYLAND_DISPLAY"));
    println!("backend      : {}", pick_backend(&Cli::parse_from(["sourizz"]), &Config::load()).id());
    println!("fonts        : {}", ok(sourizz::text::Fonts::load().available()));
    let have = |c: &str| std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| d.join(c).is_file()));
    println!("sound player : {}", ok(have("pw-play") || have("paplay") || have("aplay")));
    if session_is_wayland() {
        let groups = std::process::Command::new("id")
            .arg("-nG")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();
        println!("input group  : {} (needed on Wayland for global clicks and keys)", ok(groups.split_whitespace().any(|g| g == "input")));
    } else {
        match x11::X11::connect() {
            Ok(_) => println!("X11          : ok      (XInput2, RandR, Shape, 32-bit visual)"),
            Err(e) => println!("X11          : MISSING ({e:#})"),
        }
    }
    println!("config       : {}", Config::path().display());
    println!("daemon       : {}", if ipc::is_running() { "running" } else { "not running" });
    Ok(())
}
