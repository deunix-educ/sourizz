//! End-to-end check of the X11 backend against a running `sourizz` daemon.
//!
//! It only ever acts on its own test window: the pointer is moved over it and the
//! click lands on it (which also proves the overlay is click-through).
//!
//!   cargo run --example x11_probe -- /path/to/output-dir

#![allow(clippy::chunks_exact_to_as_chunks)]

use std::time::Duration;
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::shape::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{self, ConnectionExt as _};
use x11rb::protocol::xtest::ConnectionExt as _;

type R<T> = Result<T, Box<dyn std::error::Error>>;

struct Overlay {
    id: u32,
    x: i16,
    y: i16,
    w: u16,
    h: u16,
}

fn wait(ms: u64) {
    std::thread::sleep(Duration::from_millis(ms));
}

fn main() -> R<()> {
    let out = std::env::args().nth(1).expect("usage: x11_probe <outdir>");
    std::fs::create_dir_all(&out)?;
    let (conn, sn) = x11rb::connect(None)?;
    let root = conn.setup().roots[sn].root;
    let net_wm_name = conn.intern_atom(false, b"_NET_WM_NAME")?.reply()?.atom;
    let mut failures = 0;
    let mut check = |name: &str, ok: bool, detail: String| {
        println!(
            "[{}] {name}{}",
            if ok { " ok " } else { "FAIL" },
            if detail.is_empty() { String::new() } else { format!("  ({detail})") }
        );
        if !ok {
            failures += 1;
        }
    };

    // 1. The overlay windows exist and look right.
    let mut overlays = Vec::new();
    for child in conn.query_tree(root)?.reply()?.children {
        let name = conn.get_property(false, child, net_wm_name, xproto::AtomEnum::ANY, 0, 64)?.reply()?.value;
        if name == b"sourizz-overlay" {
            let g = conn.get_geometry(child)?.reply()?;
            let a = conn.get_window_attributes(child)?.reply()?;
            let rects = conn.shape_get_rectangles(child, shape::SK::INPUT)?.reply()?.rectangles;
            check(
                &format!("overlay {child:#x} is override-redirect, depth 32, mapped"),
                a.override_redirect && g.depth == 32 && a.map_state == xproto::MapState::VIEWABLE,
                format!("{}x{}+{}+{} depth {}", g.width, g.height, g.x, g.y, g.depth),
            );
            check(&format!("overlay {child:#x} input region is empty (click-through)"), rects.is_empty(), format!("{} rects", rects.len()));
            overlays.push(Overlay { id: child, x: g.x, y: g.y, w: g.width, h: g.height });
        }
    }
    check("at least one overlay window", !overlays.is_empty(), String::new());
    let Some(ov) = overlays.first() else { std::process::exit(1) };

    // 2. A test window of our own, placed on the first overlay's monitor.
    let (tx, ty, tw, th) = (ov.x + 400, ov.y + 400, 240u16, 140u16);
    let win = conn.generate_id()?;
    conn.create_window(
        x11rb::COPY_DEPTH_FROM_PARENT,
        win,
        root,
        tx,
        ty,
        tw,
        th,
        0,
        xproto::WindowClass::INPUT_OUTPUT,
        0,
        &xproto::CreateWindowAux::new()
            .override_redirect(1)
            .background_pixel(0x00305080)
            .event_mask(xproto::EventMask::BUTTON_PRESS | xproto::EventMask::BUTTON_RELEASE),
    )?;
    conn.map_window(win)?;
    conn.flush()?;
    wait(200);

    let grab = |ov: &Overlay| -> R<Vec<u8>> {
        let img = conn.get_image(xproto::ImageFormat::Z_PIXMAP, ov.id, 0, 0, ov.w, ov.h, !0)?.reply()?;
        Ok(img.data)
    };
    let alpha_at = |data: &[u8], ov: &Overlay, x: i32, y: i32| -> u8 {
        let (lx, ly) = (x - ov.x as i32, y - ov.y as i32);
        data[((ly as usize * ov.w as usize) + lx as usize) * 4 + 3]
    };
    let count = |data: &[u8]| data.chunks(4).filter(|p| p[3] > 0).count();
    // Move the pointer and make sure it arrived (a real mouse movement could otherwise
    // fight with the synthetic one and make the checks below meaningless).
    let warp = |x: i16, y: i16| -> R<()> {
        for _ in 0..25 {
            conn.xtest_fake_input(xproto::MOTION_NOTIFY_EVENT, 0, 0, root, x, y, 0)?;
            conn.flush()?;
            wait(40);
            let q = conn.query_pointer(root)?.reply()?;
            if q.root_x == x && q.root_y == y {
                return Ok(());
            }
        }
        Err("pointer would not stay where the probe put it (is the mouse being moved?)".into())
    };

    // 3. Pointer at the test window centre: the ring must appear around it.
    let (cx, cy) = (tx as i32 + 120, ty as i32 + 70);
    warp(cx as i16, cy as i16)?;
    wait(250);
    let before = grab(ov)?;
    let ring_px = count(&before);
    // Default ring: 56px diameter, so the stroke is at radius ~28 from the pointer.
    let on_ring = alpha_at(&before, ov, cx + 28, cy);
    let centre = alpha_at(&before, ov, cx, cy);
    let far = alpha_at(&before, ov, cx + 150, cy);
    check(
        "ring is drawn around the pointer",
        on_ring > 150 && centre == 0 && far == 0,
        format!("ring a={on_ring}, centre a={centre}, far a={far}, {ring_px} px"),
    );
    save_png(&format!("{out}/overlay_ring.png"), ov, &before)?;

    // 4. Click-through: the pointer's window is ours, and a click reaches it.
    let q = conn.query_pointer(root)?.reply()?;
    check("pointer is over the test window, not the overlay", q.child == win, format!("child={:#x}", q.child));
    conn.xtest_fake_input(xproto::BUTTON_PRESS_EVENT, 1, 0, root, 0, 0, 0)?;
    conn.flush()?;
    wait(120);
    let during = grab(ov)?;
    conn.xtest_fake_input(xproto::BUTTON_RELEASE_EVENT, 1, 0, root, 0, 0, 0)?;
    conn.flush()?;
    wait(60);
    let mut got_click = false;
    while let Some(ev) = conn.poll_for_event()? {
        if matches!(ev, Event::ButtonPress(_)) {
            got_click = true;
        }
    }
    check("click reached the window beneath the overlay", got_click, String::new());
    check("click effect adds pixels", count(&during) > ring_px + 20, format!("{} -> {}", ring_px, count(&during)));
    save_png(&format!("{out}/overlay_click.png"), ov, &during)?;

    // 5. The ripple finishes and disappears.
    wait(900);
    let after = grab(ov)?;
    check("click effect is gone afterwards", count(&after).abs_diff(ring_px) < 40, format!("{} px vs {}", count(&after), ring_px));

    // 6. Moving the pointer moves the ring and erases the old one.
    let (nx, ny) = (cx + 60, cy + 30);
    warp(nx as i16, ny as i16)?;
    wait(250);
    let moved = grab(ov)?;
    check("ring followed the pointer", alpha_at(&moved, ov, nx + 28, ny) > 150, String::new());
    check("old ring position was erased", alpha_at(&moved, ov, cx + 28, cy) == 0 && alpha_at(&moved, ov, cx - 28, cy) == 0, String::new());

    // 7. Fast sweep: no stale ring pixels left along the path.
    for i in 0..40 {
        warp((cx + 40 + i * 5) as i16, (cy - 40 + i) as i16)?;
        wait(8);
    }
    let (ex, ey) = (cx + 40 + 39 * 5, cy - 40 + 39);
    wait(300);
    let swept = grab(ov)?;
    let ring_after = count(&swept);
    check(
        "no ghost pixels after a fast sweep",
        ring_after.abs_diff(ring_px) < 60,
        format!("{ring_after} px vs {ring_px} for a single ring"),
    );
    check("ring is at the sweep end", alpha_at(&swept, ov, ex + 28, ey) > 150, String::new());

    conn.destroy_window(win)?;
    conn.flush()?;
    if failures > 0 {
        eprintln!("{failures} check(s) failed");
        std::process::exit(1);
    }
    println!("all checks passed");
    Ok(())
}

fn save_png(path: &str, ov: &Overlay, bgra: &[u8]) -> R<()> {
    let mut pm = tiny_skia::Pixmap::new(ov.w as u32, ov.h as u32).ok_or("pixmap")?;
    for (d, s) in pm.data_mut().chunks_exact_mut(4).zip(bgra.chunks_exact(4)) {
        d.copy_from_slice(&[s[2], s[1], s[0], s[3]]);
    }
    // Crop around the pointer area so the image stays small.
    let (x, y, w, h) = (300u32, 300u32, 520u32, 340u32);
    let crop = pm.clone_rect(tiny_skia::IntRect::from_xywh(x as i32, y as i32, w, h).ok_or("rect")?).ok_or("crop")?;
    let mut bg = tiny_skia::Pixmap::new(w, h).ok_or("bg")?;
    bg.fill(tiny_skia::Color::from_rgba8(70, 80, 100, 255));
    bg.draw_pixmap(0, 0, crop.as_ref(), &tiny_skia::PixmapPaint::default(), tiny_skia::Transform::identity(), None);
    bg.save_png(path)?;
    Ok(())
}
