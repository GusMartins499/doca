use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;

pub const DELAY: Duration = Duration::from_millis(260);
pub const GAP: i32 = 10;

/// Where a label of `size` sits so it is centred over an item and clear of the bar.
///
/// The bar is centred on the screen and an item near either end would push its
/// label off the edge, so the label is kept inside the screen rather than
/// centred at any cost.
pub fn place(
    item: (i32, i32),
    item_width: i32,
    size: (i32, i32),
    screen_width: i32,
) -> (i32, i32) {
    let (item_x, item_y) = item;
    let centred = item_x + (item_width - size.0) / 2;
    let x = centred.clamp(crate::dock::SCREEN_MARGIN, (screen_width - size.0 - crate::dock::SCREEN_MARGIN).max(crate::dock::SCREEN_MARGIN));
    (x, item_y - size.1 - GAP)
}

/// The dock's own label for the icon under the pointer.
///
/// GTK's tooltip is not usable here: it re-queries as the widget under the
/// pointer changes size, and the magnification lens changes that size on every
/// motion event, so the tooltip blinks for as long as the pointer rests on an
/// icon. This one is told what to show and stays put until it is told
/// otherwise.
#[derive(Clone)]
pub struct Tooltip {
    window: gtk::Window,
    label: gtk::Label,
    showing: Rc<RefCell<Option<String>>>,
    generation: Rc<Cell<u64>>,
}

impl Tooltip {
    pub fn new() -> Self {
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

        let label = gtk::Label::new(None);
        label.set_widget_name("tooltip-label");
        let frame = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        frame.set_widget_name("tooltip");
        frame.add(&label);
        window.add(&frame);
        // Shown while the window is still hidden, so the window has something
        // to be the size of. A widget that was never shown counts for nothing
        // in what its parent asks for, and an empty GTK window asks for 200
        // square — which is where this label went, over nothing, invisible.
        frame.show_all();

        Self {
            window,
            label,
            showing: Rc::new(RefCell::new(None)),
            generation: Rc::new(Cell::new(0)),
        }
    }

    /// Show `text` over `item`, after the pause that keeps a passing pointer quiet.
    ///
    /// Asking again for the label already on screen is ignored, so an item that
    /// resizes under the pointer — and so re-enters itself — does not restart
    /// the pause or move the label.
    pub fn point_at(&self, anchor: (i32, i32), width: i32, text: &str) {
        if self.showing.borrow().as_deref() == Some(text) {
            return;
        }
        *self.showing.borrow_mut() = Some(text.to_string());

        let generation = self.generation.get() + 1;
        self.generation.set(generation);

        let tooltip = self.clone();
        let text = text.to_string();
        glib::timeout_add_local_once(DELAY, move || {
            if tooltip.generation.get() != generation {
                tracing::info!("tooltip cancelled before it showed");
                return;
            }
            tooltip.show_now(&text, anchor, width);
        });
    }

    fn show_now(&self, text: &str, anchor: (i32, i32), item_width: i32) {
        self.label.set_text(text);
        let (_, natural) = self.window.preferred_size();
        let screen_width = gdk::Display::default()
            .and_then(|display| display.primary_monitor())
            .map(|monitor| monitor.geometry().width())
            .unwrap_or(natural.width);

        let (x, y) = place(
            anchor,
            item_width,
            (natural.width, natural.height),
            screen_width,
        );
        tracing::info!(x, y, w = natural.width, h = natural.height, "tooltip shown");
        self.window.move_(x, y);
        self.window.show();
        self.window.move_(x, y);
    }

    pub fn hide(&self) {
        self.generation.set(self.generation.get() + 1);
        *self.showing.borrow_mut() = None;
        self.window.hide();
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    const SCREEN: i32 = 1920;

    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_label_is_the_size_of_its_own_words() {
        let tooltip = Tooltip::new();
        tooltip.label.set_text("Brave Web Browser");

        let (_, natural) = tooltip.window.preferred_size();

        assert!(
            natural.width < 200 && natural.height < 60,
            "a {}x{} label is GTK's empty-window default, not these words — \
             the label is placed from this, and would sit over nothing",
            natural.width,
            natural.height
        );
    }

    #[test]
    fn a_label_sits_centred_over_the_icon_it_names() {
        let (x, _) = place((900, 1000), 64, (120, 30), SCREEN);

        assert_eq!(x + 60, 900 + 32);
    }

    #[test]
    fn a_label_sits_clear_above_the_bar_rather_than_over_it() {
        let (_, y) = place((900, 1000), 64, (120, 30), SCREEN);

        assert!(y + 30 < 1000, "the label would cover the icon it names");
        assert_eq!(y, 1000 - 30 - GAP);
    }

    #[test]
    fn a_label_for_the_first_icon_is_not_pushed_off_the_left_edge() {
        let (x, _) = place((10, 1000), 48, (300, 30), SCREEN);

        assert!(x >= crate::dock::SCREEN_MARGIN);
    }

    #[test]
    fn a_label_for_the_last_icon_is_not_pushed_off_the_right_edge() {
        let (x, _) = place((1880, 1000), 48, (300, 30), SCREEN);

        assert!(x + 300 <= SCREEN - crate::dock::SCREEN_MARGIN);
    }

    #[test]
    fn a_label_wider_than_the_screen_still_starts_on_it() {
        let (x, _) = place((900, 1000), 48, (4000, 30), SCREEN);

        assert_eq!(x, crate::dock::SCREEN_MARGIN);
    }
}
