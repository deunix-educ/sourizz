//! Click inside a window of ours, given its X id and a position relative to its client area:
//!   click_at <window-id-hex> <x> <y>
//! It first checks the pointer really ends up over that window (so a stray click can never hit
//! another application) and refuses otherwise.
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{self, ConnectionExt as _};
use x11rb::protocol::xtest::ConnectionExt as _;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let win = u32::from_str_radix(a[1].trim_start_matches("0x"), 16).expect("window id");
    let (rx, ry): (i16, i16) = (a[2].parse().unwrap(), a[3].parse().unwrap());
    let (conn, sn) = x11rb::connect(None).unwrap();
    let root = conn.setup().roots[sn].root;
    let tr = conn.translate_coordinates(win, root, rx, ry).unwrap().reply().unwrap();
    // The window manager wraps our window in frames; the one directly under the root is what
    // `query_pointer().child` reports.
    let mut top = win;
    loop {
        let parent = conn.query_tree(top).unwrap().reply().unwrap().parent;
        if parent == root || parent == 0 {
            break;
        }
        top = parent;
    }
    conn.xtest_fake_input(xproto::MOTION_NOTIFY_EVENT, 0, 0, root, tr.dst_x, tr.dst_y, 0).unwrap();
    conn.flush().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(120));
    let q = conn.query_pointer(root).unwrap().reply().unwrap();
    if q.child != top {
        eprintln!("REFUSED: pointer is over {:#x}, not our window {:#x} (top-level {:#x})", q.child, win, top);
        std::process::exit(2);
    }
    for t in [xproto::BUTTON_PRESS_EVENT, xproto::BUTTON_RELEASE_EVENT] {
        conn.xtest_fake_input(t, 1, 0, root, 0, 0, 0).unwrap();
        conn.flush().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(60));
    }
}
