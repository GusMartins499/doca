//! The Appearance tab: the look of the dock, applied as it is changed.
//!
//! There is no Apply and no OK. A control moves, the key goes on the bus, the
//! daemon saves and announces, and the dock changes — which is the only
//! confirmation worth having. A dialog that asked the user to press OK to see
//! what they had just chosen would be the TOML again, with buttons.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use doca_ipc::{appearance_key as key, Appearance};
use gtk::prelude::*;
use zbus::zvariant::Value;

/// Where a moved control sends its one key.
///
/// A closure rather than the bus link itself, so the tab holds no opinion
/// about D-Bus — and so a test can watch what the controls send without a
/// daemon to send it to. The guard against writing back what was just read is
/// the kind of thing that works until it doesn't; this makes it observable.
pub type Send = Rc<dyn Fn(&'static str, Value<'static>)>;

/// Whether the controls are being filled in from the daemon right now.
///
/// Setting a widget's value fires its own `changed` handler, which would send
/// back the value that had just arrived — harmless once, a loop the moment
/// something else is also writing. Every handler asks this first.
#[derive(Clone, Default)]
struct Filling(Rc<Cell<bool>>);

impl Filling {
    /// Do something to the widgets without their handlers answering back.
    fn while_filling(&self, fill: impl FnOnce()) {
        self.0.set(true);
        fill();
        self.0.set(false);
    }

    fn is_filling(&self) -> bool {
        self.0.get()
    }
}

pub struct Tab {
    pub root: gtk::Widget,
    theme: gtk::ComboBoxText,
    icon_size: gtk::Adjustment,
    magnification: gtk::Adjustment,
    magnify: gtk::Switch,
    auto_hide: gtk::Switch,
    show_trash: gtk::Switch,
    filling: Filling,
    /// Kept because the lens switch makes it insensitive, and wiring happens
    /// after the widgets are built.
    lens_slider: RefCell<Option<gtk::Scale>>,
}

/// The lens value a switch turned on should land on.
///
/// Turning magnification on has to mean something, and the slider it enables
/// may well be sitting at 1.0 — which is off. This is the config's own
/// default, so the switch agrees with a fresh install.
const LENS_WHEN_ON: f64 = 1.6;

/// Whether the lens counts as on at this scale.
pub fn lens_is_on(magnification: f64) -> bool {
    magnification > doca_ipc::MIN_MAGNIFICATION
}

/// What to send when the lens switch is flipped.
///
/// Off is unambiguous. On has to avoid sending 1.0 straight back, which would
/// read as the switch refusing to stay on.
pub fn lens_for(on: bool, slider: f64) -> f64 {
    if !on {
        return doca_ipc::MIN_MAGNIFICATION;
    }
    if lens_is_on(slider) {
        slider
    } else {
        LENS_WHEN_ON
    }
}

impl Tab {
    pub fn new() -> Self {
        let filling = Filling::default();
        let grid = gtk::Grid::new();
        grid.set_row_spacing(10);
        grid.set_column_spacing(16);
        grid.set_margin(18);

        let theme = gtk::ComboBoxText::new();
        for name in doca_ipc::THEMES {
            theme.append(Some(name), &pretty(name));
        }
        let icon_size = gtk::Adjustment::new(
            48.0,
            doca_ipc::MIN_ICON_SIZE as f64,
            doca_ipc::MAX_ICON_SIZE as f64,
            1.0,
            8.0,
            0.0,
        );
        let magnification = gtk::Adjustment::new(
            LENS_WHEN_ON,
            doca_ipc::MIN_MAGNIFICATION,
            doca_ipc::MAX_MAGNIFICATION,
            0.05,
            0.1,
            0.0,
        );
        let magnify = gtk::Switch::new();
        let auto_hide = gtk::Switch::new();
        let show_trash = gtk::Switch::new();

        let mut row = 0;
        row = add(&grid, row, "Theme", &theme, Some(
            "system takes its colours from the GTK theme you set in GNOME Tweaks.",
        ));

        // The spin and the slider share one adjustment, so they are two ways
        // of holding the same number rather than two numbers to keep in step.
        let size = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let size_slider = gtk::Scale::new(gtk::Orientation::Horizontal, Some(&icon_size));
        size_slider.set_draw_value(false);
        size_slider.set_hexpand(true);
        size.pack_start(&size_slider, true, true, 0);
        size.pack_start(&gtk::SpinButton::new(Some(&icon_size), 1.0, 0), false, false, 0);
        row = add(&grid, row, "Icon size", &size, Some(
            "Shrinks further on its own when more apps are open than fit.",
        ));

        let lens = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        lens.pack_start(&magnify, false, false, 0);
        let lens_slider = gtk::Scale::new(gtk::Orientation::Horizontal, Some(&magnification));
        lens_slider.set_draw_value(true);
        lens_slider.set_value_pos(gtk::PositionType::Right);
        lens_slider.set_digits(2);
        lens_slider.set_hexpand(true);
        lens.pack_start(&lens_slider, true, true, 0);
        row = add(&grid, row, "Magnification", &lens, Some(
            "How much the icon under the pointer grows.",
        ));

        row = add(&grid, row, "Hide automatically", &auto_hide, Some(
            "On, the dock slides away and gives maximised windows the screen back, \
             leaving a sliver to point at. Off, it keeps its space reserved.",
        ));
        add(&grid, row, "Show the trash", &show_trash, None);

        let tab = Self {
            root: grid.upcast(),
            theme,
            icon_size,
            magnification,
            magnify,
            auto_hide,
            show_trash,
            filling,
            lens_slider: RefCell::new(None),
        };
        tab.lens_slider.replace(Some(lens_slider));
        tab
    }

    /// Connect the controls to whatever is listening.
    pub fn wire(&self, send: Send) {
        let lens_slider = self
            .lens_slider
            .borrow()
            .clone()
            .expect("the slider is built before the tab is wired");
        let writing = send.clone();
        let filling = self.filling.clone();
        self.theme.connect_changed(move |theme| {
            if filling.is_filling() {
                return;
            }
            if let Some(name) = theme.active_id() {
                writing(key::THEME, Value::from(name.to_string()));
            }
        });

        let writing = send.clone();
        let filling = self.filling.clone();
        self.icon_size.connect_value_changed(move |size| {
            if filling.is_filling() {
                return;
            }
            writing(key::ICON_SIZE, Value::from(size.value().round() as i32));
        });

        let writing = send.clone();
        let filling = self.filling.clone();
        let switch = self.magnify.clone();
        self.magnification.connect_value_changed(move |lens| {
            if filling.is_filling() || !switch.is_active() {
                return;
            }
            writing(key::MAGNIFICATION, Value::from(lens.value()));
        });

        let writing = send.clone();
        let filling = self.filling.clone();
        let lens = self.magnification.clone();
        let slider = lens_slider.clone();
        self.magnify.connect_state_set(move |_, on| {
            slider.set_sensitive(on);
            if filling.is_filling() {
                return glib::Propagation::Proceed;
            }
            let value = lens_for(on, lens.value());
            // Turning it on off a slider sitting at 1.0 moves the slider too,
            // or the switch would say on while the number says off.
            if on && lens.value() != value {
                filling.while_filling(|| lens.set_value(value));
            }
            writing(key::MAGNIFICATION, Value::from(value));
            glib::Propagation::Proceed
        });

        for (switch, name) in [
            (&self.auto_hide, key::AUTO_HIDE),
            (&self.show_trash, key::SHOW_TRASH),
        ] {
            let writing = send.clone();
            let filling = self.filling.clone();
            switch.connect_state_set(move |_, on| {
                if !filling.is_filling() {
                    writing(name, Value::from(on));
                }
                glib::Propagation::Proceed
            });
        }
    }

    /// Show what the daemon says the look is, without writing it back.
    ///
    /// Called when the window opens and on every `ConfigChanged`, so a key
    /// bound to `SetAppearance` or a second window moves these controls too.
    pub fn show(&self, appearance: &Appearance) {
        self.filling.while_filling(|| {
            self.theme.set_active_id(Some(&appearance.theme));
            self.icon_size.set_value(appearance.icon_size as f64);
            let on = lens_is_on(appearance.magnification);
            self.magnify.set_active(on);
            self.magnify.set_state(on);
            if on {
                self.magnification.set_value(appearance.magnification);
            }
            for (switch, on) in [
                (&self.auto_hide, appearance.auto_hide),
                (&self.show_trash, appearance.show_trash),
            ] {
                switch.set_active(on);
                switch.set_state(on);
            }
        });
    }
}

/// A theme's name as a person reads it.
fn pretty(name: &str) -> String {
    let mut letters = name.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().collect::<String>() + letters.as_str(),
        None => String::new(),
    }
}

/// One labelled row, with the sentence that keeps it from needing a manual.
fn add(
    grid: &gtk::Grid,
    row: i32,
    label: &str,
    control: &impl IsA<gtk::Widget>,
    note: Option<&str>,
) -> i32 {
    let name = gtk::Label::new(Some(label));
    name.set_halign(gtk::Align::Start);
    grid.attach(&name, 0, row, 1, 1);

    let control = control.as_ref();
    control.set_halign(if control.is::<gtk::Switch>() {
        gtk::Align::Start
    } else {
        gtk::Align::Fill
    });
    control.set_hexpand(true);
    grid.attach(control, 1, row, 1, 1);

    let Some(note) = note else { return row + 1 };
    let hint = gtk::Label::new(Some(note));
    hint.set_widget_name("hint");
    hint.set_halign(gtk::Align::Start);
    hint.set_xalign(0.0);
    hint.set_line_wrap(true);
    hint.set_max_width_chars(52);
    hint.style_context().add_class("dim-label");
    grid.attach(&hint, 1, row + 1, 1, 1);
    row + 2
}

/// The checks that need real GTK widgets, run from `main`'s single init.
///
/// GTK belongs to the thread that starts it and the test harness gives each
/// test its own, so these cannot each be a `#[test]` — the same arrangement
/// `doca-shell` uses.
#[cfg(test)]
pub mod on_a_display {
    use super::*;
    use std::cell::RefCell;

    /// What the controls tried to put on the bus, in the order they tried.
    type Sent = Rc<RefCell<Vec<(&'static str, String)>>>;

    /// A tab wired to a record of what it tried to send.
    fn watched() -> (Tab, Sent) {
        let sent: Sent = Rc::new(RefCell::new(Vec::new()));
        let tab = Tab::new();
        let recording = sent.clone();
        tab.wire(Rc::new(move |key, value| {
            recording.borrow_mut().push((key, value.to_string()));
        }));
        (tab, sent)
    }

    fn appearance(theme: &str, icon: i32, lens: f64, hide: bool, trash: bool) -> Appearance {
        Appearance {
            theme: theme.to_string(),
            icon_size: icon,
            magnification: lens,
            auto_hide: hide,
            show_trash: trash,
            icon_theme: String::new(),
            gtk_theme: String::new(),
            cursor_theme: String::new(),
        }
    }

    /// The one that matters: reading the daemon must not write back to it.
    ///
    /// Setting a widget's value fires its own handler, so without the guard
    /// every refresh would send the value it had just received — and with two
    /// writers that is a loop, each answering the other for ever.
    pub fn showing_what_the_daemon_said_sends_nothing_back() {
        let (tab, sent) = watched();

        tab.show(&appearance("paper", 64, 2.0, true, false));
        tab.show(&appearance("midnight", 32, 1.0, false, true));

        assert!(
            sent.borrow().is_empty(),
            "the window answered its own refresh: {:?}",
            sent.borrow()
        );
    }

    pub fn the_controls_show_the_look_they_were_given() {
        let (tab, _) = watched();

        tab.show(&appearance("paper", 64, 2.0, true, false));

        assert_eq!(tab.theme.active_id().map(|id| id.to_string()).as_deref(), Some("paper"));
        assert_eq!(tab.icon_size.value() as i32, 64);
        assert_eq!(tab.magnification.value(), 2.0);
        assert!(tab.magnify.is_active(), "a lens at 2.0 is a lens that is on");
        assert!(tab.auto_hide.is_active());
        assert!(!tab.show_trash.is_active());
    }

    pub fn a_lens_turned_off_shows_as_a_switch_that_is_off() {
        let (tab, _) = watched();

        tab.show(&appearance("native", 48, MIN_MAGNIFICATION_HERE, false, true));

        assert!(!tab.magnify.is_active());
        assert!(
            tab.magnification.value() > MIN_MAGNIFICATION_HERE,
            "the slider kept 1.0, so turning the switch on would read as off"
        );
    }

    pub fn moving_a_control_sends_exactly_the_key_that_moved() {
        let (tab, sent) = watched();
        tab.show(&appearance("native", 48, 1.6, false, true));
        sent.borrow_mut().clear();

        tab.icon_size.set_value(72.0);

        let sent = sent.borrow();
        assert_eq!(sent.len(), 1, "one control moved, {} keys went out", sent.len());
        assert_eq!(sent[0].0, key::ICON_SIZE);
        assert!(sent[0].1.contains("72"), "sent {:?}", sent[0].1);
    }

    pub fn turning_the_lens_on_sends_a_scale_that_is_really_on() {
        let (tab, sent) = watched();
        tab.show(&appearance("native", 48, MIN_MAGNIFICATION_HERE, false, true));
        sent.borrow_mut().clear();

        tab.magnify.set_active(true);
        tab.magnify.set_state(true);

        let sent = sent.borrow();
        assert!(!sent.is_empty(), "the switch moved and sent nothing");
        let (name, value) = sent.last().unwrap();
        assert_eq!(*name, key::MAGNIFICATION);
        assert_ne!(
            value, "1",
            "the switch said on and sent the value that means off"
        );
    }

    /// Named here so the assertions above read in terms of the contract.
    const MIN_MAGNIFICATION_HERE: f64 = doca_ipc::MIN_MAGNIFICATION;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lens_at_one_is_a_lens_turned_off() {
        assert!(!lens_is_on(doca_ipc::MIN_MAGNIFICATION));
        assert!(lens_is_on(1.6));
        assert!(lens_is_on(doca_ipc::MAX_MAGNIFICATION));
    }

    #[test]
    fn turning_the_lens_off_sends_the_value_that_means_off() {
        assert_eq!(lens_for(false, 2.0), doca_ipc::MIN_MAGNIFICATION);
    }

    #[test]
    fn turning_the_lens_on_keeps_the_scale_the_slider_is_showing() {
        assert_eq!(lens_for(true, 2.2), 2.2);
    }

    #[test]
    fn turning_the_lens_on_from_a_slider_that_says_off_picks_a_real_scale() {
        // Sending 1.0 back would turn the switch straight off again, which
        // reads as the control refusing to work.
        let sent = lens_for(true, doca_ipc::MIN_MAGNIFICATION);

        assert!(lens_is_on(sent), "the switch said on and sent off: {sent}");
        assert_eq!(sent, LENS_WHEN_ON);
    }

    #[test]
    fn every_scale_the_switch_can_send_is_one_the_daemon_accepts() {
        for slider in [1.0, 1.5, 2.5] {
            for on in [true, false] {
                let sent = lens_for(on, slider);

                assert!(
                    (doca_ipc::MIN_MAGNIFICATION..=doca_ipc::MAX_MAGNIFICATION).contains(&sent),
                    "the window would send {sent}, which the daemon would clamp"
                );
            }
        }
    }

    #[test]
    fn every_theme_on_offer_reads_as_a_name() {
        for name in doca_ipc::THEMES {
            let shown = pretty(name);

            assert!(!shown.is_empty());
            assert_eq!(
                shown.to_lowercase(),
                name,
                "the label has to be the same theme the id names"
            );
        }
    }
}
