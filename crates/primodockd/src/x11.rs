use anyhow::{Context, Result};
use primodock_ipc::WindowInfo;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ClientMessageEvent, ConnectionExt, EventMask, Window,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;

x11rb::atom_manager! {
    pub Atoms: AtomsCookie {
        _NET_CLIENT_LIST,
        _NET_ACTIVE_WINDOW,
        _NET_CURRENT_DESKTOP,
        _NET_NUMBER_OF_DESKTOPS,
        _NET_WM_NAME,
        _NET_WM_DESKTOP,
        _NET_WM_STATE,
        _NET_WM_STATE_SKIP_TASKBAR,
        _NET_WM_WINDOW_TYPE,
        _NET_WM_WINDOW_TYPE_NORMAL,
        UTF8_STRING,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootChange {
    Windows,
    Workspace,
}

pub struct X11Backend {
    conn: RustConnection,
    root: Window,
    atoms: Atoms,
}

impl X11Backend {
    pub fn connect() -> Result<Self> {
        let (conn, screen_num) =
            x11rb::connect(None).context("cannot open an X11 display (is DISPLAY set?)")?;
        let root = conn.setup().roots[screen_num].root;
        let atoms = Atoms::new(&conn)?.reply()?;
        Ok(Self { conn, root, atoms })
    }

    fn prop_u32(&self, window: Window, property: u32) -> Result<Vec<u32>> {
        let reply = self
            .conn
            .get_property(false, window, property, AtomEnum::ANY, 0, u32::MAX)?
            .reply()?;
        Ok(reply.value32().map(|v| v.collect()).unwrap_or_default())
    }

    fn prop_text(&self, window: Window, property: u32) -> Result<Option<String>> {
        let reply = self
            .conn
            .get_property(false, window, property, AtomEnum::ANY, 0, 1024)?
            .reply()?;
        if reply.value.is_empty() {
            return Ok(None);
        }
        Ok(Some(String::from_utf8_lossy(&reply.value).into_owned()))
    }

    fn app_id(&self, window: Window) -> Result<String> {
        let reply = self
            .conn
            .get_property(false, window, AtomEnum::WM_CLASS, AtomEnum::ANY, 0, 1024)?
            .reply()?;
        let raw = String::from_utf8_lossy(&reply.value);
        let mut parts = raw.split('\0').filter(|s| !s.is_empty());
        let instance = parts.next().unwrap_or_default();
        let class = parts.next().unwrap_or(instance);
        Ok(class.to_string())
    }

    fn title(&self, window: Window) -> Result<String> {
        if let Some(name) = self.prop_text(window, self.atoms._NET_WM_NAME)? {
            if !name.is_empty() {
                return Ok(name);
            }
        }
        Ok(self
            .prop_text(window, AtomEnum::WM_NAME.into())?
            .unwrap_or_default())
    }

    fn is_dockable(&self, window: Window) -> Result<bool> {
        let types = self.prop_u32(window, self.atoms._NET_WM_WINDOW_TYPE)?;
        if !types.is_empty() && !types.contains(&self.atoms._NET_WM_WINDOW_TYPE_NORMAL) {
            return Ok(false);
        }
        let states = self.prop_u32(window, self.atoms._NET_WM_STATE)?;
        Ok(!states.contains(&self.atoms._NET_WM_STATE_SKIP_TASKBAR))
    }

    pub fn list_windows(&self) -> Result<Vec<WindowInfo>> {
        let active = self
            .prop_u32(self.root, self.atoms._NET_ACTIVE_WINDOW)?
            .first()
            .copied()
            .unwrap_or(0);
        let ids = self.prop_u32(self.root, self.atoms._NET_CLIENT_LIST)?;

        let mut out = Vec::with_capacity(ids.len());
        for id in ids {
            let Ok(dockable) = self.is_dockable(id) else {
                continue;
            };
            if !dockable {
                continue;
            }
            let workspace = self
                .prop_u32(id, self.atoms._NET_WM_DESKTOP)
                .ok()
                .and_then(|v| v.first().copied())
                .map(|w| w as i32)
                .unwrap_or(-1);
            out.push(WindowInfo {
                id,
                title: self.title(id).unwrap_or_default(),
                app_id: self.app_id(id).unwrap_or_default(),
                workspace,
                active: id == active,
            });
        }
        Ok(out)
    }

    fn send_root_message(&self, window: Window, type_: u32, data: [u32; 5]) -> Result<()> {
        let event = ClientMessageEvent::new(32, window, type_, data);
        self.conn.send_event(
            false,
            self.root,
            EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT,
            event,
        )?;
        self.conn.flush()?;
        Ok(())
    }

    pub fn activate_window(&self, id: u32) -> Result<()> {
        self.send_root_message(
            id,
            self.atoms._NET_ACTIVE_WINDOW,
            [2, x11rb::CURRENT_TIME, 0, 0, 0],
        )
    }

    pub fn current_workspace(&self) -> Result<i32> {
        Ok(self
            .prop_u32(self.root, self.atoms._NET_CURRENT_DESKTOP)?
            .first()
            .copied()
            .unwrap_or(0) as i32)
    }

    pub fn workspace_count(&self) -> Result<i32> {
        Ok(self
            .prop_u32(self.root, self.atoms._NET_NUMBER_OF_DESKTOPS)?
            .first()
            .copied()
            .unwrap_or(1) as i32)
    }

    pub fn set_workspace(&self, index: i32) -> Result<()> {
        self.send_root_message(
            self.root,
            self.atoms._NET_CURRENT_DESKTOP,
            [index as u32, x11rb::CURRENT_TIME, 0, 0, 0],
        )
    }
}

pub fn watch_root(on_change: impl Fn(RootChange) + Send + 'static) -> Result<()> {
    let backend = X11Backend::connect()?;
    std::thread::Builder::new()
        .name("x11-watch".into())
        .spawn(move || {
            if let Err(e) = watch_loop(&backend, &on_change) {
                tracing::error!("root watcher stopped: {e:#}");
            }
        })?;
    Ok(())
}

fn watch_loop(backend: &X11Backend, on_change: &impl Fn(RootChange)) -> Result<()> {
    backend.conn.change_window_attributes(
        backend.root,
        &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
    )?;
    backend.conn.flush()?;

    loop {
        let event = backend.conn.wait_for_event()?;
        let Event::PropertyNotify(e) = event else {
            continue;
        };
        if e.atom == backend.atoms._NET_CLIENT_LIST || e.atom == backend.atoms._NET_ACTIVE_WINDOW {
            on_change(RootChange::Windows);
        } else if e.atom == backend.atoms._NET_CURRENT_DESKTOP
            || e.atom == backend.atoms._NET_NUMBER_OF_DESKTOPS
        {
            on_change(RootChange::Workspace);
        }
    }
}

enum Request {
    ListWindows(async_channel::Sender<Result<Vec<WindowInfo>>>),
    ActivateWindow(u32, async_channel::Sender<Result<()>>),
    CurrentWorkspace(async_channel::Sender<Result<i32>>),
    WorkspaceCount(async_channel::Sender<Result<i32>>),
    SetWorkspace(i32, async_channel::Sender<Result<()>>),
}

#[derive(Clone)]
pub struct XHandle {
    tx: async_channel::Sender<Request>,
}

pub fn spawn_worker(backend: X11Backend) -> Result<XHandle> {
    let (tx, rx) = async_channel::unbounded::<Request>();
    std::thread::Builder::new()
        .name("x11-worker".into())
        .spawn(move || {
            while let Ok(request) = rx.recv_blocking() {
                match request {
                    Request::ListWindows(reply) => {
                        let _ = reply.send_blocking(backend.list_windows());
                    }
                    Request::ActivateWindow(id, reply) => {
                        let _ = reply.send_blocking(backend.activate_window(id));
                    }
                    Request::CurrentWorkspace(reply) => {
                        let _ = reply.send_blocking(backend.current_workspace());
                    }
                    Request::WorkspaceCount(reply) => {
                        let _ = reply.send_blocking(backend.workspace_count());
                    }
                    Request::SetWorkspace(index, reply) => {
                        let _ = reply.send_blocking(backend.set_workspace(index));
                    }
                }
            }
            tracing::debug!("x11 worker stopped");
        })?;
    Ok(XHandle { tx })
}

impl XHandle {
    async fn ask<T>(
        &self,
        build: impl FnOnce(async_channel::Sender<Result<T>>) -> Request,
    ) -> Result<T> {
        let (reply_tx, reply_rx) = async_channel::bounded(1);
        self.tx
            .send(build(reply_tx))
            .await
            .map_err(|_| anyhow::anyhow!("x11 worker is gone"))?;
        reply_rx
            .recv()
            .await
            .map_err(|_| anyhow::anyhow!("x11 worker dropped the reply"))?
    }

    pub async fn list_windows(&self) -> Result<Vec<WindowInfo>> {
        self.ask(Request::ListWindows).await
    }

    pub async fn activate_window(&self, id: u32) -> Result<()> {
        self.ask(|reply| Request::ActivateWindow(id, reply)).await
    }

    pub async fn current_workspace(&self) -> Result<i32> {
        self.ask(Request::CurrentWorkspace).await
    }

    pub async fn workspace_count(&self) -> Result<i32> {
        self.ask(Request::WorkspaceCount).await
    }

    pub async fn set_workspace(&self, index: i32) -> Result<()> {
        self.ask(|reply| Request::SetWorkspace(index, reply)).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stub_worker() -> XHandle {
        let (tx, rx) = async_channel::unbounded::<Request>();
        std::thread::spawn(move || {
            while let Ok(request) = rx.recv_blocking() {
                match request {
                    Request::ListWindows(reply) => {
                        let _ = reply.send_blocking(Ok(vec![WindowInfo {
                            id: 1,
                            title: "a window".into(),
                            app_id: "app".into(),
                            workspace: 0,
                            active: true,
                        }]));
                    }
                    Request::CurrentWorkspace(reply) => {
                        let _ = reply.send_blocking(Ok(3));
                    }
                    _ => {}
                }
            }
        });
        XHandle { tx }
    }

    #[test]
    fn x11_requests_are_served_without_any_async_runtime_present() {
        let handle = stub_worker();

        let windows = futures_lite::future::block_on(handle.list_windows()).unwrap();
        let workspace = futures_lite::future::block_on(handle.current_workspace()).unwrap();

        assert_eq!(windows.len(), 1);
        assert_eq!(workspace, 3);
    }

    #[test]
    fn a_request_whose_worker_has_gone_away_fails_instead_of_hanging() {
        let (tx, rx) = async_channel::unbounded::<Request>();
        drop(rx);
        let handle = XHandle { tx };

        let result = futures_lite::future::block_on(handle.list_windows());

        assert!(result.is_err());
    }
}

