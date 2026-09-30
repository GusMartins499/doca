//! Move the pointer, so hover can be checked on a headless sandbox.
//!
//! The magnification lens and the dock's own labels only answer to pointer
//! motion, and `scripts/sandbox.sh --headless` has no one to move it:
//!
//! ```text
//! DISPLAY=:9 cargo run -p primodockd --example warp-pointer -- 700 1035
//! ```
//!
//! It nudges the pointer a pixel at a time — one jump lands on an icon without
//! ever crossing it — and then holds still long enough to be screenshotted.
//!
//! ```text
//! warp-pointer <x> <y>            hover there and hold
//! warp-pointer <x> <y> <to>       sweep across, for measuring frame rates
//! warp-pointer <x> <y> click[:3]  hover, then press a button for real
//! ```
use x11rb::connection::Connection;
use x11rb::protocol::xproto::ConnectionExt;
use x11rb::protocol::xtest::ConnectionExt as TestExt;

const STEPS: i16 = 12;
const STEP: std::time::Duration = std::time::Duration::from_millis(40);
const SWEEP_STEP: std::time::Duration = std::time::Duration::from_millis(5);
const HOLD: std::time::Duration = std::time::Duration::from_millis(2500);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(x), Some(y)) = (args.next(), args.next()) else {
        eprintln!("usage: warp-pointer <x> <y>");
        std::process::exit(2);
    };
    let (x, y): (i16, i16) = (x.parse()?, y.parse()?);

    let (conn, screen) = x11rb::connect(None)?;
    let root = conn.setup().roots[screen].root;

    // `warp-pointer <from> <y> <to>` sweeps instead of nudging, for measuring
    // how fast the bar can actually follow a pointer.
    if let Some(to) = std::env::args().nth(3).and_then(|v| v.parse::<i16>().ok()) {
        let mut at = x;
        while at != to {
            at += if to > at { 1 } else { -1 };
            conn.warp_pointer(x11rb::NONE, root, 0, 0, 0, 0, at, y)?;
            conn.flush()?;
            std::thread::sleep(SWEEP_STEP);
        }
        return Ok(());
    }
    for step in 0..STEPS {
        conn.warp_pointer(x11rb::NONE, root, 0, 0, 0, 0, x + step, y)?;
        conn.flush()?;
        std::thread::sleep(STEP);
    }

    if let Some(click) = std::env::args().nth(3).filter(|a| a.starts_with("click")) {
        let button: u8 = click.split(':').nth(1).and_then(|b| b.parse().ok()).unwrap_or(1);
        std::thread::sleep(STEP);
        conn.xtest_fake_input(4, button, 0, root, 0, 0, 0)?; // ButtonPress
        conn.flush()?;
        std::thread::sleep(STEP);
        conn.xtest_fake_input(5, button, 0, root, 0, 0, 0)?; // ButtonRelease
        conn.flush()?;
        std::thread::sleep(HOLD);
        return Ok(());
    }

    let at = conn.query_pointer(root)?.reply()?;
    println!("pointer at {},{}", at.root_x, at.root_y);
    std::thread::sleep(HOLD);
    Ok(())
}
