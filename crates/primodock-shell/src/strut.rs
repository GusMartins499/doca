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
    )?
    .check()
    .context("setting _NET_WM_STRUT_PARTIAL")?;

    conn.change_property32(
        PropMode::REPLACE,
        xid,
        strut_atom,
        AtomEnum::CARDINAL,
        &legacy,
    )?
    .check()
    .context("setting _NET_WM_STRUT")?;

    conn.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use x11rb::protocol::xproto::{CreateWindowAux, WindowClass};

    fn read_back(xid: u32) -> Vec<u32> {
        let (conn, _) = x11rb::connect(None).unwrap();
        let atom = conn
            .intern_atom(false, b"_NET_WM_STRUT_PARTIAL")
            .unwrap()
            .reply()
            .unwrap()
            .atom;
        let reply = conn
            .get_property(false, xid, atom, AtomEnum::CARDINAL, 0, 12)
            .unwrap()
            .reply()
            .unwrap();
        reply.value32().map(|v| v.collect()).unwrap_or_default()
    }

    #[test]
    #[ignore = "needs an X display; run inside scripts/dev-session.sh"]
    fn a_strut_outlives_the_connection_that_set_it() {
        let (conn, screen_num) = x11rb::connect(None).unwrap();
        let screen = &conn.setup().roots[screen_num];
        let window = conn.generate_id().unwrap();
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            window,
            screen.root,
            0,
            0,
            10,
            10,
            0,
            WindowClass::INPUT_OUTPUT,
            screen.root_visual,
            &CreateWindowAux::new(),
        )
        .unwrap();
        conn.flush().unwrap();

        apply(
            window,
            &BottomStrut {
                height: 84,
                start_x: 493,
                end_x: 787,
            },
        )
        .unwrap();

        assert_eq!(
            read_back(window),
            vec![0, 0, 0, 84, 0, 0, 0, 0, 0, 0, 493, 787]
        );
    }
}
