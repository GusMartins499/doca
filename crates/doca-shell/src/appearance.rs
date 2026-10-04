//! The dock wearing what the desktop is wearing — or what the config says instead.
//!
//! Icons and the cursor already followed the system, because nothing here ever
//! set them: `gtk::IconTheme::default()` reads `gtk-icon-theme-name` and the
//! windows define no cursor of their own. What was missing was a way to say
//! *otherwise* for the dock alone, and a way to notice the desktop changing
//! its mind without being restarted.
//!
//! The GTK theme is the one that was genuinely ignored: the bar's stylesheet
//! is injected at `STYLE_PROVIDER_PRIORITY_APPLICATION`, over whatever the
//! theme would have given. The `system` theme in `theme.rs` is what gives it
//! back, and this module makes sure the theme those named colours are read
//! from is the one actually in force.

use std::cell::RefCell;
use std::path::PathBuf;

use gtk::prelude::*;

/// A `GtkSettings` name the dock may override, and the two values behind it.
struct Slot {
    property: &'static str,
    /// What an installed theme of this kind looks like on disk: the directory
    /// it lives under, and the file or folder inside it that proves it is one.
    installed: Installed,
    /// What the desktop asked for, with no override of ours on top.
    system: RefCell<Option<String>>,
    /// What we last wrote, so a change from outside can be told from our own.
    ours: RefCell<Option<String>>,
}

pub struct Themes {
    settings: gtk::Settings,
    slots: [Slot; 3],
}

pub const GTK_THEME: &str = "gtk-theme-name";
pub const ICON_THEME: &str = "gtk-icon-theme-name";
pub const CURSOR_THEME: &str = "gtk-cursor-theme-name";

/// How to tell whether a theme of one kind is installed.
///
/// Each kind is a directory named after the theme, holding one thing that says
/// what it is: a GTK3 theme has `gtk-3.0/gtk.css`, an icon theme an
/// `index.theme`, a cursor theme a `cursors` folder.
#[derive(Clone, Copy)]
pub struct Installed {
    /// `themes` or `icons` — the directory kind under each search root.
    pub under: &'static str,
    pub proof: &'static str,
}

const GTK_INSTALLED: Installed = Installed {
    under: "themes",
    proof: "gtk-3.0/gtk.css",
};
const ICON_INSTALLED: Installed = Installed {
    under: "icons",
    proof: "index.theme",
};
const CURSOR_INSTALLED: Installed = Installed {
    under: "icons",
    proof: "cursors",
};

/// Everywhere a theme of this kind may be installed, most personal first.
///
/// `~/.themes` and `~/.icons` are the old per-user places and are still what
/// GNOME Tweaks writes into; the XDG data directories are the rest.
pub fn roots(installed: Installed) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = glib::home_dir().to_str().map(PathBuf::from) {
        roots.push(home.join(format!(".{}", installed.under)));
    }
    roots.push(glib::user_data_dir().join(installed.under));
    for shared in glib::system_data_dirs() {
        roots.push(shared.join(installed.under));
    }
    roots
}

/// Whether a theme by this name is installed in any of these places.
///
/// Taken apart from the lookup so it can be checked against directories a test
/// makes, rather than against whatever happens to be installed on the machine
/// running it.
pub fn found(roots: &[PathBuf], name: &str, installed: Installed) -> bool {
    !name.is_empty()
        && roots
            .iter()
            .any(|root| root.join(name).join(installed.proof).exists())
}

impl Themes {
    /// Remember what the desktop wanted, before anyone overrides it.
    ///
    /// Captured once, at startup: once the dock writes a theme name of its
    /// own, the value it replaced is not recoverable from GTK, and clearing
    /// the override has to put something back.
    pub fn capture() -> Option<Self> {
        let settings = gtk::Settings::default()?;
        let slots = [
            (GTK_THEME, GTK_INSTALLED),
            (ICON_THEME, ICON_INSTALLED),
            (CURSOR_THEME, CURSOR_INSTALLED),
        ]
        .map(|(property, installed)| Slot {
            property,
            installed,
            system: RefCell::new(read(&settings, property)),
            ours: RefCell::new(None),
        });
        Some(Self { settings, slots })
    }

    /// Put the config's overrides in force, and give back what it no longer
    /// claims.
    ///
    /// Called before the stylesheet is loaded, every time: the `system` theme
    /// reads named colours out of the GTK theme, so loading the CSS first
    /// would read them from the theme on its way out.
    pub fn apply(&self, appearance: &doca_ipc::Appearance) {
        let wanted = [
            appearance.gtk_theme.as_str(),
            appearance.icon_theme.as_str(),
            appearance.cursor_theme.as_str(),
        ];

        for (slot, override_name) in self.slots.iter().zip(wanted) {
            let wanted = if override_name.is_empty() {
                slot.system.borrow().clone()
            } else {
                Some(override_name.to_string())
            };

            if read(&self.settings, slot.property) == wanted {
                continue;
            }
            // A name no theme answers to is not worth refusing — GTK falls
            // back to something readable on its own. It is worth *saying*:
            // the dock simply not changing looks identical to the setting not
            // working, and the name is usually a typo the log can point at.
            if !override_name.is_empty()
                && !found(&roots(slot.installed), override_name, slot.installed)
            {
                tracing::warn!(
                    property = slot.property,
                    theme = override_name,
                    "no theme by that name is installed; the dock keeps the one it has"
                );
            }
            tracing::info!(
                property = slot.property,
                theme = wanted.as_deref().unwrap_or("(the system's)"),
                "theme override applied"
            );
            self.settings
                .set_property(slot.property, wanted.clone());
            *slot.ours.borrow_mut() = wanted;
        }
    }

    /// Take in a change that came from the desktop rather than from us.
    ///
    /// Without this, a theme switched in GNOME Tweaks while an override is in
    /// force would be forgotten, and clearing the override later would put
    /// back the theme the session happened to start with.
    pub fn absorb(&self, property: &str) {
        let Some(slot) = self
            .slots
            .iter()
            .find(|slot| slot.property == property)
        else {
            return;
        };
        let current = read(&self.settings, property);
        if current == *slot.ours.borrow() {
            return;
        }
        tracing::info!(
            property,
            theme = current.as_deref().unwrap_or("(none)"),
            "the desktop changed its theme"
        );
        *slot.system.borrow_mut() = current;
    }
}

fn read(settings: &gtk::Settings, property: &str) -> Option<String> {
    settings.property::<Option<String>>(property)
}

/// What a change of theme asks the bar to redo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Redo {
    /// The named colours moved, so the `system` stylesheet means something else.
    pub stylesheet: bool,
    /// The icons on screen were loaded from a theme that is no longer current.
    pub icons: bool,
}

impl Redo {
    pub fn anything(&self) -> bool {
        self.stylesheet || self.icons
    }
}

/// Which of the two a given `GtkSettings` change calls for.
///
/// Kept apart from the GTK plumbing because the interesting part is the
/// reasoning, and the reasoning is testable: a cursor change asks for nothing,
/// an icon change asks for icons whatever the dock's own theme is, and a GTK
/// theme change only matters to a dock that borrows its colours.
pub fn redo_for(property: &str, theme: &str) -> Redo {
    match property {
        GTK_THEME => Redo {
            // Every other theme writes its own colours, so the GTK theme
            // underneath makes no difference to what the bar looks like.
            stylesheet: crate::theme::resolve(theme) == crate::theme::SYSTEM,
            icons: false,
        },
        ICON_THEME => Redo {
            stylesheet: false,
            // The icons come from `IconTheme::default()` no matter which
            // stylesheet the bar is wearing.
            icons: true,
        },
        // GTK draws the cursor; there is nothing for the dock to redo.
        CURSOR_THEME => Redo::default(),
        _ => Redo::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme;

    /// Built against directories the check makes, not against whatever is
    /// installed on the machine running it — which is the only way this can
    /// mean the same thing on a developer's desktop and in a container.
    #[test]
    fn a_theme_is_found_by_the_thing_that_proves_it_is_one() {
        let root = std::env::temp_dir().join(format!("doca-themes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Sweet-Dark-v40/gtk-3.0")).unwrap();
        std::fs::write(root.join("Sweet-Dark-v40/gtk-3.0/gtk.css"), "").unwrap();
        // A directory with the right name and nothing in it is not a theme.
        std::fs::create_dir_all(root.join("Half-There")).unwrap();
        let roots = [root.clone()];

        assert!(found(&roots, "Sweet-Dark-v40", GTK_INSTALLED));
        assert!(!found(&roots, "Half-There", GTK_INSTALLED));
        assert!(!found(&roots, "Sweet-Drak-v40", GTK_INSTALLED), "a typo was accepted");
        // An empty name is not a missing theme, it is no override at all.
        assert!(!found(&roots, "", GTK_INSTALLED));

        let _ = std::fs::remove_dir_all(&root);
    }

    /// Each kind is proved by a different thing, and a cursor theme is not an
    /// icon theme even though both live under `icons`.
    #[test]
    fn each_kind_of_theme_is_proved_by_its_own_contents() {
        let root = std::env::temp_dir().join(format!("doca-icons-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Flatery-Dark")).unwrap();
        std::fs::write(root.join("Flatery-Dark/index.theme"), "").unwrap();
        std::fs::create_dir_all(root.join("McMojave-cursors/cursors")).unwrap();
        let roots = [root.clone()];

        assert!(found(&roots, "Flatery-Dark", ICON_INSTALLED));
        assert!(!found(&roots, "Flatery-Dark", CURSOR_INSTALLED), "no cursors in it");
        assert!(found(&roots, "McMojave-cursors", CURSOR_INSTALLED));
        assert!(!found(&roots, "McMojave-cursors", ICON_INSTALLED), "no index.theme");

        let _ = std::fs::remove_dir_all(&root);
    }

    /// The search has to include the places GNOME Tweaks actually writes to,
    /// or every theme a person installed by hand would be called missing.
    #[test]
    fn the_search_covers_the_per_user_directories_as_well_as_the_shared_ones() {
        let places = roots(ICON_INSTALLED);

        assert!(
            places.iter().any(|root| root.ends_with(".icons")),
            "~/.icons is where Tweaks puts them: {places:?}"
        );
        assert!(
            places.iter().any(|root| root.starts_with("/usr/share")),
            "the system's own are missing: {places:?}"
        );
    }

    #[test]
    fn a_new_gtk_theme_restyles_a_dock_that_borrows_its_colours() {
        let redo = redo_for(GTK_THEME, theme::SYSTEM);

        assert!(redo.stylesheet, "the named colours the sheet reads have moved");
        assert!(!redo.icons);
    }

    #[test]
    fn a_new_gtk_theme_leaves_a_dock_with_its_own_palette_alone() {
        for name in ["native", "midnight", "paper"] {
            assert_eq!(
                redo_for(GTK_THEME, name),
                Redo::default(),
                "{name} writes its own colours and should not be rebuilt for GTK's"
            );
        }
    }

    #[test]
    fn a_new_icon_theme_reloads_the_icons_whatever_the_bar_is_wearing() {
        for name in [theme::SYSTEM, "native", "midnight", "paper"] {
            let redo = redo_for(ICON_THEME, name);

            assert!(redo.icons, "{name} draws icons from the system theme too");
            assert!(!redo.stylesheet);
        }
    }

    #[test]
    fn a_new_cursor_is_gtks_business_and_nobody_elses() {
        assert_eq!(redo_for(CURSOR_THEME, theme::SYSTEM), Redo::default());
        assert!(!redo_for(CURSOR_THEME, theme::SYSTEM).anything());
    }

    #[test]
    fn a_setting_the_dock_does_not_follow_asks_for_nothing() {
        assert!(!redo_for("gtk-font-name", theme::SYSTEM).anything());
    }

    #[test]
    fn an_unknown_theme_name_is_resolved_before_it_is_judged() {
        // A config asking for a theme that does not exist falls back to
        // `native`, which has its own colours — so GTK changing underneath
        // it is not a reason to restyle.
        assert_eq!(redo_for(GTK_THEME, "rocket"), Redo::default());
    }
}
