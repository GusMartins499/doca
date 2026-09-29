//! Reserving screen edge space on X11.
//!
//! GTK3 is used here rather than GTK4 on purpose: GTK4 dropped
//! `GDK_WINDOW_TYPE_HINT_DOCK` and closed the escape hatches to the
//! underlying X11 window, which is precisely what a dock needs.

use anyhow::{Context, Result};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, PropMode};
use x11rb::wrapper::ConnectionExt as _;

/// The area the bar occupies along the bottom edge.
pub struct BottomStrut {
    pub height: u32,
    pub start_x: u32,
    pub end_x: u32,
}

/// Tells the window manager to keep other windows out of the bar's space.
///
/// Both properties are set: `_NET_WM_STRUT_PARTIAL` is what modern window
/// managers read, and `_NET_WM_STRUT` is the older four-value form kept as
/// a fallback. Mutter honours the partial one.
pub fn apply(xid: u32, strut: &BottomStrut) -> Result<()> {
    let (conn, _) = x11rb::connect(None).context("cannot open an X11 display")?;

    let strut_partial = conn
        .intern_atom(false, b"_NET_WM_STRUT_PARTIAL")?
        .reply()?
        .atom;
    let strut_atom = conn.intern_atom(false, b"_NET_WM_STRUT")?.reply()?.atom;

    // left, right, top, bottom,
    // left_start_y, left_end_y, right_start_y, right_end_y,
    // top_start_x, top_end_x, bottom_start_x, bottom_end_x
    let partial: [u32; 12] = [
        0,
        0,
        0,
        strut.height,
        0,
        0,
        0,
        0,
        0,
        0,
        strut.start_x,
        strut.end_x,
    ];
    let legacy: [u32; 4] = [0, 0, 0, strut.height];

    conn.change_property32(
        PropMode::REPLACE,
        xid,
        strut_partial,
        AtomEnum::CARDINAL,
        &partial,
    )?;
    conn.change_property32(
        PropMode::REPLACE,
        xid,
        strut_atom,
        AtomEnum::CARDINAL,
        &legacy,
    )?;
    conn.flush()?;
    Ok(())
}
