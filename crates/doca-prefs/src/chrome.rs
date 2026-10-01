//! The small pieces every tab is built out of.
//!
//! Here rather than in whichever tab happened to need them first: two tabs
//! laying out a labelled row slightly differently is the kind of difference
//! nobody chooses, and the reader of a settings window notices.

use std::cell::Cell;
use std::rc::Rc;

use gtk::prelude::*;

/// Whether the controls are being filled in from the daemon right now.
///
/// Setting a widget's value fires its own `changed` handler, which would send
/// back the value that had just arrived. Every handler that answers a control
/// the daemon also writes to asks this first.
#[derive(Clone, Default)]
pub struct Filling(Rc<Cell<bool>>);

impl Filling {
    /// Do something to the widgets without their handlers answering back.
    pub fn while_filling(&self, fill: impl FnOnce()) {
        self.0.set(true);
        fill();
        self.0.set(false);
    }

    pub fn is_filling(&self) -> bool {
        self.0.get()
    }
}

pub fn scrolling(inner: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    let scrolled =
        gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    scrolled.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scrolled.set_vexpand(true);
    scrolled.set_shadow_type(gtk::ShadowType::In);
    scrolled.add(inner);
    scrolled
}

pub fn titled(label: &str, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let name = gtk::Label::new(Some(label));
    name.set_xalign(0.0);
    name.set_size_request(80, -1);
    row.pack_start(&name, false, false, 0);
    row.pack_start(control.as_ref(), true, true, 0);
    row
}

pub fn section(label: &str) -> gtk::Label {
    let name = gtk::Label::new(None);
    name.set_markup(&format!("<b>{}</b>", glib::markup_escape_text(label)));
    name.set_xalign(0.0);
    name.set_margin_top(6);
    name
}

pub fn hint(note: &str) -> gtk::Label {
    let hint = gtk::Label::new(Some(note));
    hint.set_widget_name("hint");
    hint.set_xalign(0.0);
    hint.set_line_wrap(true);
    hint.set_max_width_chars(52);
    hint.style_context().add_class("dim-label");
    hint
}

/// A list of names as a sentence reads it: "Work", "Work and Home",
/// "Work, Home and Personal".
pub fn listed(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [front @ .., last] => format!("{} and {last}", front.join(", ")),
    }
}

/// An id as a person reads it: `time-progress` becomes `Time progress`.
pub fn pretty(id: &str) -> String {
    let spaced = id.replace(['-', '_'], " ");
    let mut letters = spaced.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().collect::<String>() + letters.as_str(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_name_is_just_the_name() {
        assert_eq!(listed(&["Work".to_string()]), "Work");
    }

    #[test]
    fn two_names_are_joined_by_the_word_and() {
        assert_eq!(listed(&["Work".to_string(), "Home".to_string()]), "Work and Home");
    }

    #[test]
    fn more_names_take_commas_up_to_the_last_one() {
        let names = ["Work", "Home", "Games"].map(str::to_string);

        assert_eq!(listed(&names), "Work, Home and Games");
    }

    #[test]
    fn no_names_is_an_empty_sentence_rather_than_the_word_and() {
        assert_eq!(listed(&[]), "");
    }

    #[test]
    fn a_hyphenated_id_reads_as_words() {
        assert_eq!(pretty("time-progress"), "Time progress");
        assert_eq!(pretty("clock"), "Clock");
        assert_eq!(pretty(""), "");
    }
}
