use std::time::Duration;

use doca_ipc::{Body, WidgetState};

use crate::config::{NoteSettings, WidgetSettings};

use super::Widget;

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

    /// The note whole, and the paper it is on.
    ///
    /// Not split into a first line and a rest any more. That split was a
    /// guess about how much room the bar had, made by the one part of this
    /// that cannot see the bar — and the panel needs the text unsplit anyway,
    /// because it is the thing you edit it in.
    fn poll(&mut self) -> WidgetState {
        WidgetState::new(
            "note",
            Body::Note(doca_ipc::Note {
                text: self.settings.text.clone(),
                colour: doca_ipc::note_colour::resolve(&self.settings.colour).to_string(),
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

    fn body_of(state: &WidgetState) -> doca_ipc::Note {
        match state.body().expect("the body reads back") {
            Body::Note(note) => note,
            other => panic!("the note sent {other:?}"),
        }
    }

    fn settings(text: &str, colour: &str) -> NoteSettings {
        NoteSettings {
            text: text.to_string(),
            colour: colour.to_string(),
        }
    }

    #[test]
    fn a_note_rewritten_elsewhere_is_the_note_the_widget_shows() {
        let mut note = Note::new(settings("old", "yellow"));
        assert_eq!(body_of(&note.poll()).text, "old");

        note.adopt(&WidgetSettings {
            note: settings("new", "yellow"),
            ..WidgetSettings::default()
        });

        assert_eq!(body_of(&note.poll()).text, "new");
    }

    /// The note crosses whole, newlines and all.
    ///
    /// It used to arrive split into a first line and a rest. That split was a
    /// guess about how much room the bar had, made by the one part of this
    /// that cannot see the bar — and the panel needs it unsplit anyway,
    /// because the panel is where it is edited.
    #[test]
    fn the_note_arrives_as_it_was_written() {
        let written = "Dentist\nThursday at four\nbring the card";

        assert_eq!(body_of(&Note::new(settings(written, "blue")).poll()).text, written);
    }

    /// An empty note is still a note. What it *says* about being empty is the
    /// bar's to write, because the bar is the one with a square to write it
    /// in.
    #[test]
    fn an_empty_note_is_sent_empty_rather_than_filled_in() {
        assert_eq!(body_of(&Note::new(settings("", "pink")).poll()).text, "");
    }

    /// A colour nothing knows is the default rather than a tile drawn in
    /// nothing: the config is a file people edit by hand, so "yelow" has to
    /// land somewhere sensible.
    #[test]
    fn a_paper_nobody_sells_is_the_one_everybody_has() {
        assert_eq!(body_of(&Note::new(settings("x", "yelow")).poll()).colour, "yellow");
        assert_eq!(body_of(&Note::new(settings("x", "")).poll()).colour, "yellow");
        assert_eq!(body_of(&Note::new(settings("x", "purple")).poll()).colour, "purple");
    }
}
