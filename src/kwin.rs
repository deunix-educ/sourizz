//! Exact pointer position on KDE Plasma (Wayland), through a tiny KWin script.
//!
//! KWin exposes `workspace.cursorPos` to its scripts but offers no D-Bus call for it. So the
//! daemon owns a D-Bus name (`org.sourizz.Daemon`), loads a script into KWin through the
//! `org.kde.KWin /Scripting` interface, and the script calls our `Pos(x, y)` method whenever
//! the pointer moved. Nothing is sent while the pointer is still.

use crate::control::Hub;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const SCRIPT_NAME: &str = "sourizz-pointer";

/// The KWin script (KWin 6 JavaScript API: `workspace`, `callDBus`, `QTimer`).
pub const SCRIPT: &str = r#"
var last = { x: -1000000, y: -1000000 };
function send() {
    var p = workspace.cursorPos;
    if (p.x === last.x && p.y === last.y) { return; }
    last = { x: p.x, y: p.y };
    callDBus("org.sourizz.Daemon", "/Pointer", "org.sourizz.Pointer", "Pos", p.x | 0, p.y | 0);
}
// The signal gives low latency; the timer is a safety net should the signal not fire.
workspace.cursorPosChanged.connect(send);
var poll = new QTimer();
poll.interval = 16;
poll.timeout.connect(send);
poll.start();
send();
"#;

struct PointerService {
    latest: Arc<Mutex<Option<(f32, f32)>>>,
    hub: Hub,
}

#[zbus::interface(name = "org.sourizz.Pointer")]
impl PointerService {
    fn pos(&self, x: i32, y: i32) {
        if let Ok(mut l) = self.latest.lock() {
            *l = Some((x as f32, y as f32));
        }
        self.hub.poke();
    }
}

pub struct KwinFeed {
    latest: Arc<Mutex<Option<(f32, f32)>>>,
    conn: zbus::blocking::Connection,
}

impl KwinFeed {
    /// Try to hook into KWin. Returns `None` on any other desktop (or when scripting is off).
    pub fn start(hub: Hub) -> Option<KwinFeed> {
        let latest = Arc::new(Mutex::new(None));
        let conn = zbus::blocking::connection::Builder::session()
            .ok()?
            .name("org.sourizz.Daemon")
            .ok()?
            .serve_at("/Pointer", PointerService { latest: latest.clone(), hub })
            .ok()?
            .build()
            .ok()?;
        let feed = KwinFeed { latest, conn };
        match feed.load_script() {
            Ok(()) => Some(feed),
            Err(e) => {
                log::debug!("KWin script not loaded: {e}");
                None
            }
        }
    }

    fn script_path() -> PathBuf {
        std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .unwrap_or_else(std::env::temp_dir)
            .join("sourizz")
            .join("pointer.js")
    }

    fn scripting(&self, method: &str, body: &(impl serde::Serialize + zbus::zvariant::DynamicType)) -> zbus::Result<zbus::Message> {
        self.conn.call_method(Some("org.kde.KWin"), "/Scripting", Some("org.kde.kwin.Scripting"), method, body)
    }

    fn load_script(&self) -> Result<(), String> {
        let path = Self::script_path();
        std::fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&path, SCRIPT).map_err(|e| e.to_string())?;
        // A previous run that crashed may have left the script loaded.
        let _ = self.scripting("unloadScript", &(SCRIPT_NAME,));
        let reply = self.scripting("loadScript", &(path.to_string_lossy().to_string(), SCRIPT_NAME)).map_err(|e| e.to_string())?;
        let id: i32 = reply.body().deserialize().map_err(|e| e.to_string())?;
        if id < 0 {
            return Err("KWin refused to load the script".into());
        }
        self.conn
            .call_method(Some("org.kde.KWin"), format!("/Scripting/Script{id}").as_str(), Some("org.kde.kwin.Script"), "run", &())
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// The most recent position reported by the script, if it changed since the last call.
    pub fn take_latest(&self) -> Option<(f32, f32)> {
        self.latest.lock().ok().and_then(|mut l| l.take())
    }
}

impl Drop for KwinFeed {
    fn drop(&mut self) {
        let _ = self.scripting("unloadScript", &(SCRIPT_NAME,));
    }
}
