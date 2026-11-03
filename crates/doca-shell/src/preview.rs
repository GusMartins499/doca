//! The windows of an app, shown over its icon once the pointer rests there
//! (#18, P3: "preview de janela no hover").
//!
//! The name is the tooltip's job and comes first; resting longer turns it
//! into this — one card for each window, its picture and its title, and a
//! click on a card brings that window forward. The right click still lists
//! the same windows by title, for the keyboard and for a pointer that does
//! not wait.
//!
//! It has to be reachable, which a tooltip never is: the pointer leaves the
//! icon to get to it. So leaving does not close it straight away — it waits
//! [`motion::PREVIEW_STAYS`] for the pointer to arrive, and arriving cancels
//! the wait. Another icon is another answer and closes it at once: the
//! pointer is on the bar, not on its way here.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use doca_ipc::{DocaProxy, DockItem, WindowInfo};
use gtk::prelude::*;

use crate::motion;
use crate::tooltip::Tooltip;

/// How large a window's picture is drawn, in logical pixels. A window is
/// fitted inside this in proportion, so the cards line up whatever shape
/// the windows are.
pub const SHOT: (i32, i32) = (200, 125);

/// The icon that stands in for a window with nothing on screen to show.
const STAND_IN: i32 = 48;

/// The room one card takes along the row: the picture, the card's padding
/// and the gap to the next one. The same figures as `#preview-window` and
/// the box the cards sit in.
const CARD: i32 = SHOT.0 + 2 * 6 + 6;

/// How many cards fit across a screen `width` wide, never fewer than one: a
/// preview cut off at the edge is worse than one that shows fewer windows,
/// and the right click still lists them all.
pub fn how_many_fit(width: i32) -> usize {
    let room = width - 2 * crate::dock::SCREEN_MARGIN - 2 * 10;
    (room / CARD).max(1) as usize
}

/// A window's picture as the daemon sent it, or `None` when there is none or
/// the bytes are not the picture they say they are.
pub fn picture_of(width: u32, height: u32, rgba: Vec<u8>) -> Option<gdk::gdk_pixbuf::Pixbuf> {
    let (width, height) = (i32::try_from(width).ok()?, i32::try_from(height).ok()?);
    if width == 0 || height == 0 || rgba.len() != width as usize * height as usize * 4 {
        return None;
    }
    Some(gdk::gdk_pixbuf::Pixbuf::from_bytes(
        &glib::Bytes::from_owned(rgba),
        gdk::gdk_pixbuf::Colorspace::Rgb,
        true,
        8,
        width,
        height,
        width * 4,
    ))
}

#[derive(Clone)]
pub struct Preview {
    window: gtk::Window,
    heading: gtk::Label,
    cards: gtk::Box,
    label: Tooltip,
    proxy: Rc<DocaProxy<'static>>,
    /// The item shown, or waiting to be: `None` is nothing pending at all.
    showing: Rc<RefCell<Option<String>>>,
    /// Bumped by anything that makes a wait or a reply out of date.
    generation: Rc<Cell<u64>>,
    leaving: Rc<RefCell<Option<glib::SourceId>>>,
}

impl Preview {
    pub fn new(label: &Tooltip, proxy: Rc<DocaProxy<'static>>) -> Self {
        let window = gtk::Window::new(gtk::WindowType::Popup);
        window.set_type_hint(gdk::WindowTypeHint::Tooltip);
        window.set_keep_above(true);
        window.set_decorated(false);
        window.set_resizable(false);
        window.set_accept_focus(false);
        window.set_app_paintable(true);
        if let Some(visual) = gtk::prelude::WidgetExt::screen(&window).and_then(|s| s.rgba_visual())
        {
            window.set_visual(Some(&visual));
        }
        window.add_events(gdk::EventMask::ENTER_NOTIFY_MASK | gdk::EventMask::LEAVE_NOTIFY_MASK);

        let heading = gtk::Label::new(None);
        heading.set_widget_name("tooltip-label");
        heading.set_xalign(0.0);
        let cards = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let column = gtk::Box::new(gtk::Orientation::Vertical, 6);
        column.add(&heading);
        column.add(&cards);
        // The tooltip's frame, so the preview is the same surface the name
        // was on a moment ago, grown — not a second kind of popup.
        let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
        frame.set_widget_name("tooltip");
        frame.add(&column);
        window.add(&frame);
        frame.show_all();

        let preview = Self {
            window,
            heading,
            cards,
            label: label.clone(),
            proxy,
            showing: Rc::default(),
            generation: Rc::new(Cell::new(0)),
            leaving: Rc::default(),
        };
        let arriving = preview.clone();
        preview.window.connect_enter_notify_event(move |_, _| {
            arriving.stay();
            glib::Propagation::Proceed
        });
        let going = preview.clone();
        preview.window.connect_leave_notify_event(move |_, event| {
            if event.detail() != gdk::NotifyType::Inferior {
                going.let_go();
            }
            glib::Propagation::Proceed
        });
        preview
    }

    /// The popup itself, for whatever has to know while it is up.
    pub fn surface(&self) -> &gtk::Window {
        &self.window
    }

    pub fn is_up(&self) -> bool {
        self.window.is_visible()
    }

    /// The pointer is on `item`, drawn `width` wide with its top-left corner
    /// at `anchor` on the screen — or on nothing, for `None`.
    pub fn over(&self, item: Option<(&DockItem, (i32, i32), i32)>) {
        let Some((item, anchor, width)) = item else {
            self.let_go();
            return;
        };
        if self.showing.borrow().as_deref() == Some(item.id.as_str()) {
            self.stay();
            return;
        }
        self.hide();
        if item.windows.is_empty() {
            return;
        }
        *self.showing.borrow_mut() = Some(item.id.clone());
        let generation = self.generation.get();
        let preview = self.clone();
        let (id, name) = (item.id.clone(), item.name.clone());
        let icon = item.icon.clone();
        glib::timeout_add_local_once(motion::PREVIEW, move || {
            if preview.generation.get() != generation {
                return;
            }
            glib::spawn_future_local(async move {
                let windows = preview.proxy.item_windows(&id).await.unwrap_or_default();
                if preview.generation.get() != generation || windows.is_empty() {
                    return;
                }
                preview.show_now(&name, &icon, &windows, anchor, width);
            });
        });
    }

    /// The pointer has left for somewhere that is neither the icon nor here.
    /// A preview on screen is given a moment for the pointer to reach it;
    /// one still waiting to show is simply not shown.
    pub fn let_go(&self) {
        if !self.is_up() {
            self.hide();
            return;
        }
        if self.leaving.borrow().is_some() {
            return;
        }
        let preview = self.clone();
        let waiting = glib::timeout_add_local_once(motion::PREVIEW_STAYS, move || {
            preview.leaving.replace(None);
            preview.hide();
        });
        self.leaving.replace(Some(waiting));
    }

    fn stay(&self) {
        if let Some(waiting) = self.leaving.replace(None) {
            waiting.remove();
        }
    }

    pub fn hide(&self) {
        self.stay();
        self.generation.set(self.generation.get() + 1);
        *self.showing.borrow_mut() = None;
        if self.window.is_visible() {
            self.window.hide();
            self.label.hush(false);
        }
    }

    fn show_now(&self, name: &str, icon: &str, windows: &[WindowInfo], anchor: (i32, i32), width: i32) {
        let monitor = gdk::Display::default()
            .and_then(|display| display.monitor_at_point(anchor.0, anchor.1));
        let screen = monitor
            .as_ref()
            .map(|monitor| monitor.geometry())
            .map(|area| (area.x(), area.width()))
            .unwrap_or((0, 1920));
        let density = monitor.map(|monitor| monitor.scale_factor()).unwrap_or(1);

        self.heading.set_text(name);
        for card in self.cards.children() {
            self.cards.remove(&card);
        }
        let shown = how_many_fit(screen.1).min(windows.len());
        for window in &windows[..shown] {
            let card = self.card(window, icon, density);
            self.cards.add(&card);
        }
        self.cards.show_all();

        // The name stops being said on its own: it is the heading now.
        self.label.hush(true);
        self.window.resize(1, 1);
        let (_, natural) = self.window.preferred_size();
        let (x, y) = crate::tooltip::place(anchor, width, (natural.width, natural.height), screen);
        tracing::info!(windows = windows.len(), shown, x, y, "window preview shown");
        self.window.move_(x, y);
        self.window.show();
        self.window.move_(x, y);
    }

    /// One window: its picture once the daemon has taken it, its icon until
    /// then and for good when there is nothing on screen to take.
    fn card(&self, window: &WindowInfo, icon: &str, density: i32) -> gtk::Button {
        let picture = gtk::Image::new();
        picture.set_size_request(SHOT.0, SHOT.1);
        if let Some(stand_in) = crate::dock::load_pixbuf(icon, STAND_IN, density) {
            crate::dock::show_dense(&picture, &stand_in, density);
        }
        let title = gtk::Label::new(Some(if window.title.is_empty() {
            "Untitled window"
        } else {
            &window.title
        }));
        title.set_widget_name("tooltip-label");
        title.set_ellipsize(pango::EllipsizeMode::End);
        title.set_max_width_chars(1);
        title.set_size_request(SHOT.0, -1);
        let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
        column.add(&picture);
        column.add(&title);

        let card = gtk::Button::new();
        card.set_widget_name("preview-window");
        card.set_relief(gtk::ReliefStyle::None);
        card.set_can_focus(false);
        // No tooltip with the title: it would cover the picture to repeat
        // the line under it. The full title is the card's accessible name.
        card.add(&column);
        atk_name(&card, &window.title);

        let id = window.id;
        let choosing = self.clone();
        card.connect_clicked(move |_| {
            choosing.hide();
            let proxy = choosing.proxy.clone();
            glib::spawn_future_local(async move {
                if let Err(e) = proxy.activate_window(id).await {
                    tracing::warn!("the window chosen in the preview did not come forward: {e}");
                }
            });
        });

        let (proxy, generation, preview) = (self.proxy.clone(), self.generation.get(), self.clone());
        let room = (SHOT.0 * density) as u32;
        let room = (room, (SHOT.1 * density) as u32);
        glib::spawn_future_local(async move {
            let Ok((width, height, rgba)) = proxy.window_preview(id, room.0, room.1).await else {
                return;
            };
            if preview.generation.get() != generation {
                return;
            }
            if let Some(shot) = picture_of(width, height, rgba) {
                crate::dock::show_dense(&picture, &shot, density);
            }
        });
        card
    }
}

fn atk_name(card: &gtk::Button, title: &str) {
    use atk::prelude::*;
    if let Some(accessible) = card.accessible() {
        accessible.set_name(title);
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn a_full_hd_screen_shows_several_windows_side_by_side() {
        assert_eq!(how_many_fit(1920), 8);
    }

    #[test]
    fn a_screen_too_narrow_for_one_card_still_shows_one() {
        assert_eq!(how_many_fit(100), 1);
    }

    #[test]
    fn the_cards_that_are_shown_fit_on_the_screen_they_are_shown_on() {
        for width in [800, 1366, 1920, 2560, 5120] {
            let cards = how_many_fit(width) as i32;
            assert!(cards * CARD <= width - 2 * crate::dock::SCREEN_MARGIN, "{cards} cards at {width}");
        }
    }

    #[test]
    fn a_picture_is_taken_only_when_the_bytes_are_the_size_they_claim() {
        assert!(picture_of(2, 2, vec![0; 16]).is_some());
        assert!(picture_of(2, 2, vec![0; 15]).is_none(), "one byte short");
        assert!(picture_of(0, 0, Vec::new()).is_none(), "nothing on screen to show");
    }
}
