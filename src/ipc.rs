//! Tiny line protocol over a Unix socket so the CLI (and your desktop's custom
//! shortcuts, which is how global hotkeys work on Wayland) can drive the daemon.

use crate::control::{Command, Hub, parse_line};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

pub fn socket_path() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p| p.is_dir()) {
        Some(d) => d.join("sourizz.sock"),
        None => PathBuf::from(format!("/tmp/sourizz-{}.sock", rustix::process::getuid().as_raw())),
    }
}

/// Send one command line to the running daemon and return its reply.
pub fn send(line: &str) -> std::io::Result<String> {
    let mut s = UnixStream::connect(socket_path())?;
    s.set_read_timeout(Some(Duration::from_secs(3)))?;
    s.set_write_timeout(Some(Duration::from_secs(3)))?;
    writeln!(s, "{}", line.trim())?;
    let mut reply = String::new();
    BufReader::new(s).read_line(&mut reply)?;
    Ok(reply.trim().to_string())
}

pub fn is_running() -> bool {
    UnixStream::connect(socket_path()).is_ok()
}

/// Start the control server in a background thread.
pub fn serve(hub: Hub) -> std::io::Result<()> {
    let path = socket_path();
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            if is_running() {
                return Err(std::io::Error::new(std::io::ErrorKind::AddrInUse, "sourizz is already running"));
            }
            // Stale socket left by a crash.
            let _ = std::fs::remove_file(&path);
            UnixListener::bind(&path)?
        }
        Err(e) => return Err(e),
    };
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    std::thread::Builder::new().name("ipc".into()).spawn(move || {
        for conn in listener.incoming().flatten() {
            let hub = hub.clone();
            // One short-lived thread per client keeps a stuck client from blocking others.
            let _ = std::thread::Builder::new().name("ipc-client".into()).spawn(move || handle(conn, hub));
        }
    })?;
    Ok(())
}

pub fn cleanup() {
    let _ = std::fs::remove_file(socket_path());
}

fn handle(mut conn: UnixStream, hub: Hub) {
    let _ = conn.set_read_timeout(Some(Duration::from_secs(2)));
    let mut line = String::new();
    // Bound the line so a rogue local client cannot make us buffer without limit.
    let mut reader = BufReader::new((&conn).take_bytes(1024));
    if reader.read_line(&mut line).is_err() {
        return;
    }
    let reply = match line.split_whitespace().next() {
        Some("status") => {
            let (tx, rx) = mpsc::channel();
            hub.send(Command::Status(tx));
            rx.recv_timeout(Duration::from_secs(1)).unwrap_or_else(|_| "error: daemon busy".into())
        }
        _ => match parse_line(&line).and_then(validate) {
            Ok(cmd) => {
                hub.send(cmd);
                "ok".to_string()
            }
            Err(e) => format!("error: {e}"),
        },
    };
    let _ = writeln!(conn, "{reply}");
}

/// Reject bad `set` values here so the CLI can report them, instead of the daemon only logging.
fn validate(cmd: Command) -> Result<Command, String> {
    if let Command::SetOption(k, v) = &cmd {
        crate::control::set_option(&crate::config::Config::default(), k, v)?;
    }
    Ok(cmd)
}

trait TakeBytes {
    fn take_bytes(self, n: u64) -> std::io::Take<Self>
    where
        Self: Sized;
}

impl<R: std::io::Read> TakeBytes for R {
    fn take_bytes(self, n: u64) -> std::io::Take<Self> {
        std::io::Read::take(self, n)
    }
}
