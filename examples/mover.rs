//! Move the pointer in circles for N seconds (default 4) to generate load.
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{self};
use x11rb::protocol::xtest::ConnectionExt as _;
fn main() {
    let secs: f64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(4.0);
    let (conn, sn) = x11rb::connect(None).unwrap();
    let root = conn.setup().roots[sn].root;
    let t0 = std::time::Instant::now();
    while t0.elapsed().as_secs_f64() < secs {
        let t = t0.elapsed().as_secs_f64() * 3.0;
        let (x, y) = (960.0 + 400.0 * t.cos(), 540.0 + 250.0 * (t * 1.3).sin());
        conn.xtest_fake_input(xproto::MOTION_NOTIFY_EVENT, 0, 0, root, x as i16, y as i16, 0).unwrap();
        conn.flush().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(8));
    }
}
