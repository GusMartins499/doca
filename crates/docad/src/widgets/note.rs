use std::time::Duration;

use doca_ipc::{Body, Simple, WidgetState, NO_PROGRESS};

use crate::config::{NoteSettings, WidgetSettings};

use super::Widget;

pub fn split(text: &str) -> (String, String) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return ("—".to_string(), "empty note".to_string());
    }
    match trimmed.split_once('\n') {
        Some((first, rest)) => (first.trim().to_string(), rest.trim().replace('\n', " ")),
        None => (trimmed.to_string(), "note".to_string()),
    }
}

pub struct Note {
    settings: NoteSettings,
}

impl Note {
    pub fn new(settings: NoteSettings) -> Self {
        Self { settings }
    }
}

impl Widget for Note {
    fn id(&self) -> &str {
        "note"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(3600)
    }

    fn poll(&mut self) -> WidgetState {
        let (label, detail) = split(&self.settings.text);
        WidgetState::new(
            "note",
            Body::Simple(Simple {
                label,
                detail,
                progress: NO_PROGRESS,
                active: false,
            }),
        )
    }

    fn adopt(&mut self, settings: &WidgetSettings) {
        self.settings = settings.note.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::widgets::Drawn;

    #[test]
    fn a_note_rewritten_elsewhere_is_the_note_the_widget_shows() {
        let mut note = Note::new(NoteSettings { text: "old".into() });
        assert_eq!(note.poll().label(), "old");

        note.adopt(&WidgetSettings {
            note: NoteSettings { text: "new".into() },
            ..WidgetSettings::default()
        });

        assert_eq!(note.poll().label(), "new");
    }

    #[test]
    fn a_one_line_note_is_the_label_with_a_plain_caption() {
        assert_eq!(split("call the dentist"), ("call the dentist".into(), "note".into()));
    }

    #[test]
    fn a_two_line_note_puts_the_first_line_forward() {
        let (label, detail) = split("Dentist\nThursday at four");

        assert_eq!(label, "Dentist");
        assert_eq!(detail, "Thursday at four");
    }

    #[test]
    fn the_rest_of_a_long_note_is_flattened_onto_one_line() {
        let (_, detail) = split("Title\nsecond\nthird");

        assert_eq!(detail, "second third");
    }

    #[test]
    fn an_empty_note_says_it_is_empty_rather_than_drawing_nothing() {
        assert_eq!(split("   "), ("—".into(), "empty note".into()));
        assert_eq!(split(""), ("—".into(), "empty note".into()));
    }

    #[test]
    fn surrounding_whitespace_is_not_shown() {
        assert_eq!(split("  spaced  ").0, "spaced");
    }
}
