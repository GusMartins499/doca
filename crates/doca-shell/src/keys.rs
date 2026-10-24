//! The dock with the keyboard on it (#18, item 9).
//!
//! A key bound to `FocusDock` brings the bar up — auto-hide or not — with
//! the first icon ringed. The arrows walk the icons and then the tiles,
//! `Enter` chooses, `Menu` and `Shift+F10` offer the menu, and `Esc` gives
//! the keyboard back to the app that had it and lets the bar hide again.
//!
//! # How the keyboard gets to a dock
//!
//! The bar is `_NET_WM_WINDOW_TYPE_DOCK`, and a window manager is entitled
//! never to focus one. There were three ways to ask for the keyboard, and
//! they were measured, not guessed at (Xvfb, with openbox and with no window
//! manager at all, a DOCK window beside an ordinary one holding the focus,
//! real key presses from `xdotool`):
//!
//! - `gtk_window_present_with_time`, the `_NET_ACTIVE_WINDOW` request;
//! - `gdk_window_focus`, the same request by another road;
//! - an explicit keyboard and pointer grab on the bar's window, the way a
//!   `GtkMenu` takes the keyboard when it pops up.
//!
//! All three delivered the keys in that sandbox, and GTK counted the bar as
//! focused under each — under the grab because GDK reports a grab's
//! `NotifyGrab` focus change as the window gaining focus, which is what lets
//! a tile draw its `:focus` ring. They part company where the sandbox cannot
//! follow. The first two are requests the manager may refuse: a DOCK is
//! exactly what Mutter declines to focus, and the request comes from a key
//! the desktop handled, so it carries no user timestamp of ours and runs
//! into focus-stealing prevention besides. And they move the focus, so
//! `Esc` would have to know which window had it and ask for it back.
//!
//! The grab asks nobody. It is what the panel and the folder grid already
//! stand on over this same bar (see `panel.rs`), and letting go of it is
//! the whole of giving the keyboard back: the X focus never moved, so the
//! app that had it still has it. What it costs is what a menu's grab costs —
//! while the dock has the keyboard, a click outside it is the dock's, and
//! ends its hold on the keyboard rather than reaching the app under it.
//!
//! One thing about it the sandbox did show: a key bound on the desktop is a
//! passive grab of the desktop's, active from the press until the release.
//! A `FocusDock` that lands while the key is still down finds the keyboard
//! taken, so the grab is tried again for a moment rather than given up on
//! the first refusal.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use doca_ipc::DockItem;
use gtk::prelude::*;

use crate::row::Row;
use crate::tooltip::Tooltip;
use crate::widget_tile::Shelf;
use crate::Hide;

/// How long the keyboard is waited for when someone else has it — the key
/// that asked for the dock, still held down — and how often it is asked for.
const GRAB_RETRY: Duration = Duration::from_millis(50);
const GRAB_TRIES: u32 = 20;

/// What a key means to the dock while the dock has the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Next,
    Back,
    First,
    Last,
    Choose,
    Menu,
    Leave,
}

/// Which of the dock's keys this is, if it is one.
///
/// Left and right because the bar is a row, and Tab with them because it is
/// the key everybody tries first. Up and down are left alone: on a dock they
/// are the menu's, and the menu has keys of its own.
pub fn read(key: gdk::keys::Key, state: gdk::ModifierType) -> Option<Key> {
    use gdk::keys::constants as k;
    let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
    Some(match key {
        k::Right | k::KP_Right => Key::Next,
        k::Left | k::KP_Left => Key::Back,
        k::Tab | k::KP_Tab if !shift => Key::Next,
        k::Tab | k::KP_Tab | k::ISO_Left_Tab => Key::Back,
        k::Home | k::KP_Home => Key::First,
        k::End | k::KP_End => Key::Last,
        k::Return | k::KP_Enter | k::ISO_Enter | k::space | k::KP_Space => Key::Choose,
        k::Menu => Key::Menu,
        k::F10 if shift => Key::Menu,
        k::Escape => Key::Leave,
        _ => return None,
    })
}

/// Where a key moves the keyboard to, among `count` places.
///
/// Stopping at the ends rather than wrapping: a row read out one place at a
/// time has to be able to say "this is the last one", and a ring that jumps
/// from the trash back to the first app says nothing of the kind. `Home` and
/// `End` are the long way round.
pub fn step(at: Option<usize>, count: usize, key: Key) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let last = count - 1;
    Some(match (key, at) {
        (Key::First, _) | (_, None) => 0,
        (Key::Last, _) => last,
        (Key::Next, Some(at)) => (at + 1).min(last),
        (Key::Back, Some(at)) => at.saturating_sub(1),
        (_, Some(at)) => at.min(last),
    })
}

/// Somewhere the keyboard can stand: an icon on the row, or a tile.
///
/// By id, because the bar is rebuilt under it whenever a window opens or
/// closes, and a place counted from the left would move to the neighbour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stop {
    Icon(String),
    Tile(String),
}

/// What the dock does with an icon: chosen (a click, or `Enter`) or offered
/// its menu (a right click, or `Menu`). The row's index of the icon, the
/// item, and the click when there was one — a key has none, and a menu put
/// up for a key goes over the icon instead of under a pointer.
pub type Act = Rc<dyn Fn(usize, DockItem, Option<gdk::Event>)>;

/// The keyboard, while the dock has it.
#[derive(Clone)]
pub struct Keyboard {
    window: gtk::Window,
    row: Row,
    tiles: Rc<Shelf>,
    label: Tooltip,
    hide: Hide,
    held: Rc<Cell<bool>>,
    at: Rc<RefCell<Option<Stop>>>,
    choose: Rc<RefCell<Option<Act>>>,
    offer: Rc<RefCell<Option<Act>>>,
    /// The next try at the grab, while the keyboard is someone else's.
    waiting: Rc<RefCell<Option<glib::SourceId>>>,
}

impl Keyboard {
    pub fn new(window: &gtk::Window, row: &Row, tiles: Rc<Shelf>, label: &Tooltip, hide: Hide) -> Self {
        let keys = Self {
            window: window.clone(),
            row: row.clone(),
            tiles,
            label: label.clone(),
            hide,
            held: Rc::default(),
            at: Rc::default(),
            choose: Rc::default(),
            offer: Rc::default(),
            waiting: Rc::default(),
        };

        // On the window and not on whatever has focus: a key-press is sent
        // to the toplevel before its focus widget, so the dock's keys are
        // the dock's whichever icon or tile the ring is on — and GTK's own
        // idea of what the arrows do between focusable widgets never runs.
        let pressing = keys.clone();
        window.connect_key_press_event(move |_, event| {
            pressing.press(event.keyval(), event.state())
        });

        // A click outside the dock is the dock's while it holds the pointer
        // too, and it means the hand went somewhere else.
        let clicking = keys.clone();
        window.connect_button_press_event(move |window, event| {
            if clicking.held() && !inside(window, event.position()) {
                clicking.leave();
            }
            glib::Propagation::Proceed
        });

        // A grab is lost to one of the dock's own menus, which hands it back
        // when it closes (`watches`), or to the window going away under it
        // — a fullscreen app stepping the dock aside — which is the end.
        let breaking = keys.clone();
        window.connect_grab_broken_event(move |_, broken| {
            if breaking.held() && broken.grab_window().is_none() {
                tracing::debug!("the dock lost the keyboard");
                breaking.leave();
            }
            glib::Propagation::Proceed
        });
        keys
    }

    /// What choosing an icon and offering its menu do — the same closures a
    /// click runs, so a key cannot do something a click would not.
    pub fn acts(&self, choose: Act, offer: Act) {
        *self.choose.borrow_mut() = Some(choose);
        *self.offer.borrow_mut() = Some(offer);
    }

    pub fn held(&self) -> bool {
        self.held.get()
    }

    /// Where the keyboard is standing, if the dock has it.
    pub fn at(&self) -> Option<Stop> {
        self.at.borrow().clone()
    }

    /// Take the keyboard: the bar up, held up, and the first item ringed.
    pub fn take(&self) {
        if !self.window.is_visible() {
            // Stepped aside for a fullscreen window: there is no bar on the
            // screen to give the keyboard to.
            tracing::info!("asked for the keyboard with no bar on screen");
            return;
        }
        if !self.held.replace(true) {
            self.hide.hold();
        }
        // Straight up, not after the pointer's wait: nobody is wondering
        // whether this was meant.
        self.hide.slide(&self.window, true);
        self.window.set_focus_visible(true);
        let first = self.stops().into_iter().next();
        self.go(first);
        self.grab(GRAB_TRIES);
        tracing::info!("the dock has the keyboard");
    }

    /// Give the keyboard back, and the bar to auto-hide.
    pub fn leave(&self) {
        if !self.held.replace(false) {
            return;
        }
        if let Some(waiting) = self.waiting.replace(None) {
            waiting.remove();
        }
        if let Some(seat) = self.window.display().default_seat() {
            seat.ungrab();
        }
        self.row.ring(None);
        self.label.hide();
        self.at.replace(None);
        self.window.set_focus(None::<&gtk::Widget>);
        self.hide.release(&self.window);
        if !crate::pointer_is_over(&self.window) {
            self.hide.slide(&self.window, false);
        }
        tracing::info!("the dock gave the keyboard back");
    }

    /// Take the grab, waiting a moment for whoever has the keyboard now.
    fn grab(&self, tries: u32) {
        let (Some(surface), Some(seat)) = (self.window.window(), self.window.display().default_seat()) else {
            self.leave();
            return;
        };
        let status = seat.grab(
            &surface,
            gdk::SeatCapabilities::KEYBOARD | gdk::SeatCapabilities::POINTER,
            true,
            None,
            None,
            None,
        );
        if status == gdk::GrabStatus::Success {
            return;
        }
        if tries == GRAB_TRIES {
            // Read back by scripts/check-live.sh, to know the wait it checks
            // was really waited.
            tracing::debug!(?status, "the keyboard is someone else's; waiting for it");
        }
        if tries == 0 {
            tracing::warn!(?status, "the keyboard never came free; the dock lets it go");
            self.leave();
            return;
        }
        let trying = self.clone();
        let next = glib::timeout_add_local_once(GRAB_RETRY, move || {
            trying.waiting.replace(None);
            if trying.held() {
                trying.grab(tries - 1);
            }
        });
        if let Some(old) = self.waiting.replace(Some(next)) {
            old.remove();
        }
    }

    /// A menu of the dock's, put up while the dock has the keyboard.
    ///
    /// The menu takes the grab when it opens — it is the same client, so X
    /// hands it over — and lets go of it when it closes, which leaves the
    /// keyboard with nobody. `Esc` out of a menu is a step back to the bar,
    /// so the grab is taken again and the ring is where it was; anything
    /// else that closes one — something chosen in it, a click elsewhere —
    /// is the hand moving on, and the dock lets go too.
    pub fn watches(&self, menu: &gtk::Menu) {
        let keys = self.clone();
        menu.connect_deactivate(move |_| {
            if !keys.held() {
                return;
            }
            let escaped = gtk::current_event().is_some_and(|event| {
                event.event_type() == gdk::EventType::KeyPress
                    && event.keyval() == Some(*gdk::keys::constants::Escape)
            });
            if !escaped {
                keys.leave();
                return;
            }
            // After the menu has finished closing, which is when its grab
            // is let go of — taking it now would be taking it back from a
            // menu that is about to drop it.
            let keys = keys.clone();
            glib::idle_add_local_once(move || {
                if keys.held() {
                    keys.grab(GRAB_TRIES);
                    keys.go(keys.at());
                }
            });
        });
    }

    /// The bar was rebuilt under the keyboard: stay on the same thing if it
    /// is still there, and go to the first if it is not.
    pub fn refresh(&self) {
        if !self.held() {
            return;
        }
        let stops = self.stops();
        let here = self.at().filter(|stop| stops.contains(stop));
        self.go(here.or_else(|| stops.into_iter().next()));
    }

    /// A key, while the dock has the keyboard. Every key is the dock's then:
    /// a letter typed with the ring on an icon belongs to nobody else.
    pub fn press(&self, keyval: gdk::keys::Key, state: gdk::ModifierType) -> glib::Propagation {
        if !self.held() {
            return glib::Propagation::Proceed;
        }
        match read(keyval, state) {
            Some(Key::Leave) => self.leave(),
            Some(Key::Choose) => self.choose_here(),
            Some(Key::Menu) => self.offer_here(),
            Some(walk) => {
                let stops = self.stops();
                let at = self.at().and_then(|here| stops.iter().position(|stop| *stop == here));
                let to = step(at, stops.len(), walk).and_then(|to| stops.get(to).cloned());
                self.go(to);
            }
            None => {}
        }
        glib::Propagation::Stop
    }

    /// Every place the keyboard can stand, in the order the bar shows them.
    pub fn stops(&self) -> Vec<Stop> {
        self.row
            .items()
            .into_iter()
            .map(|item| Stop::Icon(item.id))
            .chain(self.tiles.on_the_bar().into_iter().map(|(id, _)| Stop::Tile(id)))
            .collect()
    }

    /// Put the keyboard here: the ring on it, GTK's focus on it, and on an
    /// icon its name over it, as a pointer resting there would get.
    fn go(&self, stop: Option<Stop>) {
        match &stop {
            Some(Stop::Icon(id)) => {
                self.row.ring(Some(id));
                self.row.area.grab_focus();
                if let Some(index) = self.row.index_of(id) {
                    if let Some(item) = self.row.item(index) {
                        crate::name_at(&self.row, &self.label, index, &item.name);
                    }
                }
            }
            Some(Stop::Tile(id)) => {
                self.row.ring(None);
                self.label.hide();
                if let Some((_, tile)) = self.tiles.on_the_bar().into_iter().find(|(tile, _)| tile == id) {
                    tile.root.grab_focus();
                }
            }
            None => {
                self.row.ring(None);
                self.label.hide();
            }
        }
        // Read back by scripts/check-live.sh, which presses real keys and
        // has no other way to see where they went.
        tracing::debug!(?stop, "the keyboard is on");
        self.at.replace(stop);
    }

    fn choose_here(&self) {
        match self.at() {
            Some(Stop::Icon(id)) => {
                let Some(index) = self.row.index_of(&id) else { return };
                let Some(item) = self.row.item(index) else { return };
                let choose = self.choose.borrow().clone();
                if let Some(choose) = choose {
                    choose(index, item, None);
                }
            }
            Some(Stop::Tile(id)) => self.open(&id),
            None => {}
        }
    }

    fn offer_here(&self) {
        match self.at() {
            Some(Stop::Icon(id)) => {
                let Some(index) = self.row.index_of(&id) else { return };
                let Some(item) = self.row.item(index) else { return };
                let offer = self.offer.borrow().clone();
                if let Some(offer) = offer {
                    offer(index, item, None);
                }
            }
            // A tile's panel is its menu: it has no other.
            Some(Stop::Tile(id)) => self.open(&id),
            None => {}
        }
    }

    fn open(&self, id: &str) {
        self.label.hide();
        if let Some((_, tile)) = self.tiles.on_the_bar().into_iter().find(|(tile, _)| tile == id) {
            tile.open();
        }
    }
}

/// Whether a point in a window's coordinates is inside it.
fn inside(window: &gtk::Window, (x, y): (f64, f64)) -> bool {
    x >= 0.0 && y >= 0.0 && x < window.allocated_width() as f64 && y < window.allocated_height() as f64
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use gdk::keys::constants as k;

    const NONE: gdk::ModifierType = gdk::ModifierType::empty();

    #[test]
    fn the_keys_the_item_names_are_the_keys_the_dock_reads() {
        assert_eq!(read(k::Right, NONE), Some(Key::Next));
        assert_eq!(read(k::Left, NONE), Some(Key::Back));
        assert_eq!(read(k::Return, NONE), Some(Key::Choose));
        assert_eq!(read(k::KP_Enter, NONE), Some(Key::Choose));
        assert_eq!(read(k::Menu, NONE), Some(Key::Menu));
        assert_eq!(read(k::F10, gdk::ModifierType::SHIFT_MASK), Some(Key::Menu));
        assert_eq!(read(k::F10, NONE), None, "F10 alone is the desktop's");
        assert_eq!(read(k::Escape, NONE), Some(Key::Leave));
        assert_eq!(read(k::Tab, NONE), Some(Key::Next));
        assert_eq!(read(k::ISO_Left_Tab, gdk::ModifierType::SHIFT_MASK), Some(Key::Back));
        assert_eq!(read(k::a, NONE), None);
    }

    #[test]
    fn the_ring_stops_at_both_ends_rather_than_wrapping() {
        assert_eq!(step(Some(0), 3, Key::Back), Some(0));
        assert_eq!(step(Some(2), 3, Key::Next), Some(2));
        assert_eq!(step(Some(0), 3, Key::Next), Some(1));
        assert_eq!(step(Some(1), 3, Key::Back), Some(0));
        assert_eq!(step(Some(1), 3, Key::Last), Some(2));
        assert_eq!(step(Some(2), 3, Key::First), Some(0));
    }

    #[test]
    fn a_ring_that_lost_its_place_starts_again_at_the_first() {
        assert_eq!(step(None, 3, Key::Next), Some(0));
        assert_eq!(step(None, 3, Key::Back), Some(0));
        assert_eq!(step(Some(7), 3, Key::Choose), Some(2), "past the end is the last");
        assert_eq!(step(Some(0), 0, Key::Next), None, "an empty bar has nowhere to stand");
    }

    // What needs a window, run by `crate::on_a_display`.

    fn waiting(span: Duration) {
        let until = std::time::Instant::now() + span;
        while std::time::Instant::now() < until {
            while gtk::events_pending() {
                gtk::main_iteration_do(false);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn item(id: &str, windows: usize) -> DockItem {
        DockItem {
            id: id.to_string(),
            name: id.to_string(),
            icon: "application-x-executable".to_string(),
            pinned: true,
            windows: (0..windows as u32).collect(),
            active: false,
        }
    }

    /// The ids something was asked to do with, in order.
    type Noted = Rc<RefCell<Vec<String>>>;

    struct Bar {
        window: gtk::Window,
        row: Row,
        tiles: Rc<Shelf>,
        hide: Hide,
        keys: Keyboard,
        chosen: Noted,
        offered: Noted,
        opened: Noted,
    }

    /// A dock as the real one is put together, small: three icons and two
    /// tiles in a DOCK window at the bottom of the screen, auto-hide on and
    /// the bar already gone.
    fn bar() -> Bar {
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_type_hint(gdk::WindowTypeHint::Dock);
        window.set_decorated(false);
        let items = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        let row = Row::new();
        items.add(&row.area);
        window.add(&items);
        row.fill(
            &[item("files", 0), item("firefox", 2), item("code", 1)],
            crate::row::Rest::new(48, 2, 2, 1.0),
        );

        let opened: Noted = Rc::default();
        let noting = opened.clone();
        let expand: crate::widget_tile::Expand =
            Rc::new(move |id: &str, _: &gtk::Widget| noting.borrow_mut().push(id.to_string()));
        let tiles = Rc::new(Shelf::new());
        tiles.show(
            &items,
            &[
                crate::widget_tile::tests::simple("clock"),
                crate::widget_tile::tests::simple("water"),
            ],
            crate::widget_tile::Look {
                icon_size: 48,
                palette: crate::theme::palette(crate::theme::DEFAULT),
                level: crate::fit::Level::Full,
            },
            &expand,
        );
        window.show_all();
        window.move_(100, 500);

        let hide = Hide::default();
        hide.place(&window, 100, 500, 60, true);
        hide.settle();
        assert!(!hide.shown.get(), "the bar under test is not hidden to begin with");

        let label = Tooltip::new();
        let keys = Keyboard::new(&window, &row, tiles.clone(), &label, hide.clone());
        let chosen: Noted = Rc::default();
        let offered: Noted = Rc::default();
        let (choosing, offering) = (chosen.clone(), offered.clone());
        keys.acts(
            Rc::new(move |_, item, _| choosing.borrow_mut().push(item.id)),
            Rc::new(move |_, item, _| offering.borrow_mut().push(item.id)),
        );
        waiting(Duration::from_millis(100));
        Bar { window, row, tiles, hide, keys, chosen, offered, opened }
    }

    fn press(bar: &Bar, key: gdk::keys::Key) {
        bar.keys.press(key, NONE);
        waiting(Duration::from_millis(30));
    }

    fn close(bar: Bar) {
        bar.keys.leave();
        bar.window.close();
        waiting(Duration::from_millis(30));
    }

    /// The first box of the item: the bar comes up for the keyboard with
    /// auto-hide on, the first icon has it, and the bar stays while it does.
    pub fn the_dock_takes_the_keyboard_and_keeps_the_bar_up() {
        let bar = bar();
        bar.keys.take();
        waiting(crate::motion::SLIDE * 2);

        assert!(bar.hide.shown.get(), "the bar stayed hidden for the keyboard");
        assert_eq!(bar.keys.at(), Some(Stop::Icon("files".into())), "the first item is not the one focused");
        assert_eq!(bar.row.ringed().as_deref(), Some("files"));
        assert!(
            bar.window.has_toplevel_focus(),
            "the grab did not make the bar the focused window, so nothing on it can show focus"
        );
        assert!(bar.row.area.has_focus(), "the row does not hold GTK's focus with an icon ringed");

        // The pointer leaving is no reason to hide a bar the keyboard is on.
        bar.hide.intend(&bar.window, false);
        waiting(crate::motion::HIDE_AFTER * 2);
        assert!(bar.hide.shown.get(), "the bar hid itself with the keyboard still on it");

        press(&bar, k::Escape);
        waiting(crate::motion::SLIDE * 2);
        assert!(!bar.keys.held(), "Esc did not give the keyboard back");
        assert!(!bar.hide.shown.get(), "Esc gave the keyboard back and left the bar up");
        assert_eq!(bar.row.ringed(), None, "the ring stayed on after Esc");
        assert!(
            gdk::Display::default()
                .and_then(|display| display.default_seat())
                .and_then(|seat| seat.keyboard())
                .is_some_and(|keyboard| !keyboard.display().device_is_grabbed(&keyboard)),
            "the keyboard is still grabbed after Esc"
        );
        close(bar);
    }

    /// The arrows walk the icons, then the tiles, and stop at the ends.
    pub fn the_arrows_walk_the_icons_and_then_the_tiles() {
        let bar = bar();
        bar.keys.take();
        waiting(Duration::from_millis(80));

        press(&bar, k::Left);
        assert_eq!(bar.keys.at(), Some(Stop::Icon("files".into())), "Left at the first icon went somewhere");
        press(&bar, k::Right);
        assert_eq!(bar.row.ringed().as_deref(), Some("firefox"));
        press(&bar, k::Right);
        press(&bar, k::Right);
        assert_eq!(bar.keys.at(), Some(Stop::Tile("clock".into())), "the walk did not go on to the tiles");
        assert_eq!(bar.row.ringed(), None, "an icon kept the ring with a tile focused");
        let clock = bar.tiles.on_the_bar()[0].1.root.clone();
        assert!(clock.has_focus(), "the tile the ring is on does not have GTK's focus");
        assert!(
            clock.state_flags().contains(gtk::StateFlags::FOCUSED),
            "the tile is not :focus, so no theme can draw its ring"
        );
        press(&bar, k::End);
        assert_eq!(bar.keys.at(), Some(Stop::Tile("water".into())));
        press(&bar, k::Right);
        assert_eq!(bar.keys.at(), Some(Stop::Tile("water".into())), "Right at the end wrapped round");
        press(&bar, k::Home);
        assert_eq!(bar.keys.at(), Some(Stop::Icon("files".into())));
        close(bar);
    }

    /// Enter chooses what is ringed and the menu key offers its menu — the
    /// closures a click runs — and a tile opens its panel either way.
    pub fn enter_chooses_and_the_menu_key_offers() {
        let bar = bar();
        bar.keys.take();
        waiting(Duration::from_millis(80));

        press(&bar, k::Right);
        bar.keys.press(k::F10, gdk::ModifierType::SHIFT_MASK);
        press(&bar, k::Menu);
        assert_eq!(*bar.offered.borrow(), vec!["firefox", "firefox"], "Shift+F10 and Menu offered nothing");
        press(&bar, k::Return);
        assert_eq!(*bar.chosen.borrow(), vec!["firefox"], "Enter chose nothing");

        press(&bar, k::End);
        press(&bar, k::Return);
        press(&bar, k::Menu);
        assert_eq!(*bar.opened.borrow(), vec!["water", "water"], "a tile ringed did not open its panel");
        close(bar);
    }

    /// A rebuild under the keyboard keeps the ring on the same app, wherever
    /// that app has moved to — and moves it on when the app has gone.
    pub fn a_rebuild_keeps_the_ring_on_the_same_app() {
        let bar = bar();
        bar.keys.take();
        waiting(Duration::from_millis(80));
        press(&bar, k::Right);

        let rest = crate::row::Rest::new(48, 2, 2, 1.0);
        bar.row.fill(&[item("terminal", 1), item("files", 0), item("firefox", 2), item("code", 1)], rest);
        bar.keys.refresh();
        assert_eq!(bar.row.ringed().as_deref(), Some("firefox"), "an app opening moved the ring");

        bar.row.fill(&[item("terminal", 1), item("files", 0)], rest);
        bar.keys.refresh();
        assert_eq!(bar.keys.at(), Some(Stop::Icon("terminal".into())), "the ring stayed on an app that closed");
        close(bar);
    }

    /// The panel hands the keyboard back to its tile (`1287953`); this is the
    /// bar's half: `Esc` out of a menu of the dock's gives the bar the grab
    /// back, with the ring where it was, and the bar stays up.
    pub fn esc_out_of_a_menu_comes_back_to_the_bar() {
        let bar = bar();
        bar.keys.take();
        waiting(Duration::from_millis(80));
        press(&bar, k::End);

        let menu = gtk::Menu::new();
        menu.add(&gtk::MenuItem::with_label("something"));
        menu.show_all();
        bar.hide.holds_for(&bar.window, &menu);
        bar.keys.watches(&menu);
        menu.popup_at_widget(&bar.window, gdk::Gravity::NorthWest, gdk::Gravity::SouthWest, None);
        waiting(Duration::from_millis(80));
        assert!(menu.is_visible(), "the menu under test never opened");

        // Through the main loop rather than straight at the menu, so that it
        // is the event in hand when the menu closes — which is how the bar
        // tells Esc from a click elsewhere.
        let mut escape = escape_on(&menu);
        gtk::main_do_event(&mut escape);
        waiting(Duration::from_millis(120));

        assert!(!menu.is_visible(), "Esc did not close the menu");
        assert!(bar.keys.held(), "Esc out of the menu took the keyboard off the dock");
        assert_eq!(bar.keys.at(), Some(Stop::Tile("water".into())), "the ring did not come back where it was");
        assert!(bar.window.has_toplevel_focus(), "the bar did not take the keyboard back from the menu");
        bar.hide.intend(&bar.window, false);
        waiting(crate::motion::HIDE_AFTER * 2);
        assert!(bar.hide.shown.get(), "the bar went away with the keyboard still on it");
        close(bar);
    }

    /// An Escape as the main loop would deliver it to the menu's window.
    fn escape_on(menu: &gtk::Menu) -> gdk::Event {
        use glib::translate::{IntoGlib, ToGlibPtr, ToGlibPtrMut};
        let window = menu
            .toplevel()
            .and_then(|top| top.window())
            .expect("a menu that is up has a window");
        let keycode = gdk::Keymap::for_display(&window.display())
            .and_then(|keymap| keymap.entries_for_keyval(*k::Escape).first().map(|entry| entry.keycode()))
            .expect("the keymap has no Escape");
        let mut event = gdk::Event::new(gdk::EventType::KeyPress);
        // SAFETY: a KeyPress event is a GdkEventKey; the window reference
        // handed over is the one `gdk_event_free` gives back.
        unsafe {
            let raw: *mut gdk::ffi::GdkEvent = event.to_glib_none_mut().0;
            let raw = raw as *mut gdk::ffi::GdkEventKey;
            (*raw).window = window.to_glib_full();
            (*raw).send_event = 1;
            (*raw).keyval = k::Escape.into_glib();
            (*raw).hardware_keycode = keycode as u16;
        }
        if let Some(keyboard) = window.display().default_seat().and_then(|seat| seat.keyboard()) {
            event.set_device(Some(&keyboard));
        }
        event
    }
}
