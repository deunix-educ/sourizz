//! Press a chord through XTEST: `fake_keys Control_L Alt_L Shift_L F9` (last key is the main one).
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{self, ConnectionExt as _};
use x11rb::protocol::xtest::ConnectionExt as _;

fn main() {
    let names: Vec<String> = std::env::args().skip(1).collect();
    let (conn, sn) = x11rb::connect(None).unwrap();
    let root = conn.setup().roots[sn].root;
    let (min, max) = (conn.setup().min_keycode, conn.setup().max_keycode);
    let map = conn.get_keyboard_mapping(min, max - min + 1).unwrap().reply().unwrap();
    let per = map.keysyms_per_keycode as usize;
    let code = |name: &str| -> u8 {
        let sym = sourizz::keysym::from_name(name).unwrap_or_else(|| panic!("unknown key {name}"));
        (0..(map.keysyms.len() / per))
            .find(|r| map.keysyms[r * per] == sym)
            .map(|r| min + r as u8)
            .unwrap_or_else(|| panic!("{name} not on keyboard"))
    };
    let codes: Vec<u8> = names.iter().map(|n| code(n)).collect();
    for c in &codes {
        conn.xtest_fake_input(xproto::KEY_PRESS_EVENT, *c, 0, root, 0, 0, 0).unwrap();
        conn.flush().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
    for c in codes.iter().rev() {
        conn.xtest_fake_input(xproto::KEY_RELEASE_EVENT, *c, 0, root, 0, 0, 0).unwrap();
        conn.flush().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
    }
}
