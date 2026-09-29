use anyhow::{Context, Result};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, PropMode};
use x11rb::wrapper::ConnectionExt as _;

pub struct BottomStrut {
    pub height: u32,
    pub start_x: u32,
    pub end_x: u32,
}

pub fn apply(xid: u32, strut: &BottomStrut) -> Result<()> {
    let (conn, _) = x11rb::connect(None).context("cannot open an X11 display")?;

    let strut_partial = conn
        .intern_atom(false, b"_NET_WM_STRUT_PARTIAL")?
        .reply()?
        .atom;
    let strut_atom = conn.intern_atom(false, b"_NET_WM_STRUT")?.reply()?.atom;

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
