//! The GNOME side of the Shortcuts tab: which key calls which Doca action.
//!
//! The dock does not grab keys. On Linux the keyboard belongs to the desktop,
//! so the dock puts its actions on the bus and GNOME is told which key calls
//! them — which is why this tab writes `org.gnome.settings-daemon`, and not
//! the dock's own config, and why the window says so out loud.
//!
//! [`scripts/bind-key.sh`] already did this, and still does: it is what
//! automation and a terminal reach for, and it predates this window. So the
//! names here are not a fresh design — [`slot`], [`command`] and [`label`]
//! reproduce the script's byte for byte, and a test holds them to it. The two
//! must agree, because they write the same list: a slot named differently
//! would not replace the script's binding, it would sit beside it, and one key
//! would call the action twice.
//!
//! Nothing here touches GTK. The [`Desktop`] trait is the whole of what this
//! needs from GNOME, which is what lets every rule below be tested against a
//! list in memory rather than against the machine the test runs on.

use std::collections::HashMap;

use gtk::gio::prelude::{SettingsExt, SettingsExtManual};

/// The schema that owns the list of custom keybindings on a GNOME desktop.
const MEDIA_KEYS: &str = "org.gnome.settings-daemon.plugins.media-keys";

/// The relocatable schema one binding is stored in.
const SLOT_SCHEMA: &str = "org.gnome.settings-daemon.plugins.media-keys.custom-keybinding";

const SLOT_ROOT: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings";

/// A Doca action a key can be bound to.
///
/// Only the two the dock already answers to. `CycleEnvironment` and
/// `SetEnvironment` are on the bus today; everything else #18 asks for —
/// cycling backwards, toggling auto-hide, toggling one widget — needs a method
/// that does not exist yet, and inventing the key for it here would bind a key
/// to an error reply.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Action {
    /// Step to the next dock in the order the Docks tab shows.
    Cycle,
    /// Go straight to one dock, by name.
    Switch(String),
}

/// One binding as GNOME stores it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Binding {
    pub name: String,
    pub binding: String,
    pub command: String,
}

/// The slug the script makes of a dock's name.
///
/// `tr A-Z a-z` and then `s/[^a-z0-9]\+/-/g; s/^-//; s/-$//`, which is where
/// the two have to agree exactly. Note what it means for a name with no
/// letters or digits in it at all: the slug is empty. That is not fixed here —
/// fixing it would be the disagreement — it is caught by [`bindable`].
pub fn slug(name: &str) -> String {
    let mut slug = String::new();
    let mut dashed = false;
    for letter in name.to_lowercase().chars() {
        if letter.is_ascii_lowercase() || letter.is_ascii_digit() {
            slug.push(letter);
            dashed = false;
        } else if !dashed && !slug.is_empty() {
            slug.push('-');
            dashed = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    slug
}

/// Where this action's binding is stored, as a dconf path.
///
/// One named slot per action, which is what makes binding the same action
/// twice *change* that binding rather than add a second one.
pub fn slot(action: &Action) -> String {
    match action {
        Action::Cycle => format!("{SLOT_ROOT}/doca-cycle/"),
        Action::Switch(name) => format!("{SLOT_ROOT}/doca-{}/", slug(name)),
    }
}

/// What GNOME runs when the key is pressed.
pub fn command(action: &Action) -> String {
    let call = format!(
        "gdbus call --session --dest {} --object-path {} --method {}",
        doca_ipc::BUS_NAME,
        doca_ipc::OBJECT_PATH,
        doca_ipc::INTERFACE,
    );
    match action {
        Action::Cycle => format!("{call}.CycleEnvironment"),
        // The quotes are part of the stored string, not shell syntax being
        // escaped away: GNOME hands the command to a shell, and a dock called
        // `My Work` is one argument.
        Action::Switch(name) => format!("{call}.SetEnvironment \"{name}\""),
    }
}

/// What the desktop's own keyboard settings will call this binding.
pub fn label(action: &Action) -> String {
    match action {
        Action::Cycle => "Doca: cycle environment".to_string(),
        Action::Switch(name) => format!("Doca: {name}"),
    }
}

/// Whether a key can be bound to this action at all, and why not when it
/// cannot.
///
/// Two docks can disagree by name and agree by slug — `My Work` and `My-Work`
/// both slug to `my-work` — and a name made only of punctuation slugs to
/// nothing. Either way two actions would share one slot, so binding one would
/// silently move the other. The window refuses instead, and names the dock it
/// is colliding with, because the fix is a rename and the user is one tab away
/// from it.
pub fn bindable(action: &Action, docks: &[String]) -> Result<(), String> {
    let Action::Switch(name) = action else {
        return Ok(());
    };
    if slug(name).is_empty() {
        return Err(format!(
            "{name:?} has no letters or digits to make a key name out of — \
             rename this dock to bind a key to it"
        ));
    }
    let twin = docks
        .iter()
        .find(|other| *other != name && slug(other) == slug(name));
    match twin {
        Some(twin) => Err(format!(
            "{name:?} and {twin:?} would share one keybinding — \
             rename one of them to bind a key to either"
        )),
        None => Ok(()),
    }
}

/// What GNOME has to offer, and nothing more.
///
/// A trait so that every rule above and below can be tested against a list in
/// memory. The real one is [`Gnome`]; a test's is a `HashMap`.
pub trait Desktop {
    /// The slots the desktop currently knows about, in its own order.
    fn slots(&self) -> Vec<String>;
    /// The binding in one slot, or `None` for a slot holding nothing.
    fn read(&self, slot: &str) -> Option<Binding>;
    /// Put a binding in one slot, creating it.
    fn write(&self, slot: &str, binding: &Binding);
    /// Empty one slot.
    fn erase(&self, slot: &str);
    /// Replace the list of slots.
    fn set_slots(&self, slots: &[String]);
}

/// Every binding the desktop holds, slot by slot.
///
/// Slots with nothing in them are skipped rather than reported as a binding of
/// the empty string, which would then clash with every key not yet bound.
pub fn all(desktop: &impl Desktop) -> Vec<(String, Binding)> {
    desktop
        .slots()
        .into_iter()
        .filter_map(|slot| {
            let held = desktop.read(&slot)?;
            (!held.binding.is_empty()).then_some((slot, held))
        })
        .collect()
}

/// What is bound to each Doca action right now.
pub fn bound(desktop: &impl Desktop, actions: &[Action]) -> HashMap<Action, String> {
    let held = all(desktop);
    actions
        .iter()
        .filter_map(|action| {
            let slot = slot(action);
            let (_, binding) = held.iter().find(|(at, _)| *at == slot)?;
            Some((action.clone(), binding.binding.clone()))
        })
        .collect()
}

/// Who else already answers to this key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Clash {
    /// Another Doca action — which this window owns, and can say exactly.
    Ours(Action),
    /// Something else on this desktop, named as its own settings name it.
    Theirs(String),
}

/// Whether this key is taken, looking only at the custom keybindings.
///
/// The fixed keybindings GNOME ships — `org.gnome.desktop.wm.keybindings` and
/// the shell's own — are deliberately not scanned: that is dozens of schemas,
/// each a list, and a scan that goes stale with every GNOME release. What this
/// catches is the collision a person actually creates, between keys they chose
/// themselves.
pub fn clash(
    desktop: &impl Desktop,
    key: &str,
    wanted_by: &Action,
    actions: &[Action],
) -> Option<Clash> {
    let mine = slot(wanted_by);
    let held = all(desktop);
    let (at, binding) = held
        .iter()
        .find(|(at, held)| held.binding == key && *at != mine)?;
    match actions.iter().find(|action| slot(action) == *at) {
        Some(action) => Some(Clash::Ours(action.clone())),
        None => Some(Clash::Theirs(binding.name.clone())),
    }
}

/// Bind a key to an action, keeping every other binding on the desktop.
///
/// The list of slots is read, added to and written back rather than replaced:
/// it is shared by every application on the desktop, and writing only Doca's
/// slots into it would unbind everyone else's keys. This is the part the
/// script gets right and the part that is easy to get wrong, so it is also the
/// part with the most tests behind it.
pub fn bind(desktop: &impl Desktop, action: &Action, key: &str) {
    let slot = slot(action);
    desktop.write(
        &slot,
        &Binding {
            name: label(action),
            binding: key.to_string(),
            command: command(action),
        },
    );
    let mut slots = desktop.slots();
    if !slots.contains(&slot) {
        slots.push(slot);
        desktop.set_slots(&slots);
    }
}

/// Unbind an action, leaving every other slot where it was.
pub fn unbind(desktop: &impl Desktop, action: &Action) {
    let slot = slot(action);
    let kept: Vec<String> = desktop
        .slots()
        .into_iter()
        .filter(|known| *known != slot)
        .collect();
    desktop.set_slots(&kept);
    // The slot's own keys go too. The script leaves them behind, which is
    // harmless until the day a slot is rebound and a stale name from a dock
    // that was renamed two releases ago shows up in GNOME's settings.
    desktop.erase(&slot);
}

/// GNOME itself, through GSettings.
pub struct Gnome {
    keys: gtk::gio::Settings,
}

impl Gnome {
    /// The desktop, or nothing when this is not a desktop that has one.
    ///
    /// The schema is looked up before it is opened, and that is not
    /// defensiveness for its own sake: `gio::Settings::new` on a schema the
    /// system does not install **aborts the process**. It does not return an
    /// error to handle. So on KDE, on sway, on a machine with
    /// gnome-settings-daemon not installed, asking politely first is the
    /// difference between a tab that explains itself and a settings window
    /// that vanishes when you click it.
    pub fn found() -> Option<Self> {
        let source = gtk::gio::SettingsSchemaSource::default()?;
        source.lookup(MEDIA_KEYS, true)?;
        source.lookup(SLOT_SCHEMA, true)?;
        tracing::info!("the desktop keeps custom keybindings");
        Some(Self {
            keys: gtk::gio::Settings::new(MEDIA_KEYS),
        })
    }

    fn slot_settings(&self, slot: &str) -> gtk::gio::Settings {
        gtk::gio::Settings::with_path(SLOT_SCHEMA, slot)
    }
}

impl Desktop for Gnome {
    fn slots(&self) -> Vec<String> {
        self.keys
            .strv("custom-keybindings")
            .into_iter()
            .map(|slot| slot.to_string())
            .filter(|slot| !slot.is_empty())
            .collect()
    }

    fn read(&self, slot: &str) -> Option<Binding> {
        let held = self.slot_settings(slot);
        Some(Binding {
            name: held.string("name").to_string(),
            binding: held.string("binding").to_string(),
            command: held.string("command").to_string(),
        })
    }

    fn write(&self, slot: &str, binding: &Binding) {
        let held = self.slot_settings(slot);
        let _ = held.set_string("name", &binding.name);
        let _ = held.set_string("binding", &binding.binding);
        let _ = held.set_string("command", &binding.command);
    }

    fn erase(&self, slot: &str) {
        let held = self.slot_settings(slot);
        for key in ["name", "binding", "command"] {
            held.reset(key);
        }
    }

    fn set_slots(&self, slots: &[String]) {
        let borrowed: Vec<&str> = slots.iter().map(String::as_str).collect();
        if let Err(e) = self.keys.set_strv("custom-keybindings", borrowed) {
            tracing::warn!("the desktop would not take the keybinding list: {e}");
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::cell::RefCell;

    /// A desktop that is a list in memory — including the keybindings other
    /// applications put there, which is what the interesting checks are about.
    #[derive(Default)]
    pub struct Fake {
        slots: RefCell<Vec<String>>,
        held: RefCell<HashMap<String, Binding>>,
    }

    impl Fake {
        pub fn with(bindings: &[(&str, &str, &str)]) -> Self {
            let fake = Self::default();
            for (slot, key, name) in bindings {
                fake.slots.borrow_mut().push(slot.to_string());
                fake.held.borrow_mut().insert(
                    slot.to_string(),
                    Binding {
                        name: name.to_string(),
                        binding: key.to_string(),
                        command: String::new(),
                    },
                );
            }
            fake
        }
    }

    impl Desktop for Fake {
        fn slots(&self) -> Vec<String> {
            self.slots.borrow().clone()
        }
        fn read(&self, slot: &str) -> Option<Binding> {
            self.held.borrow().get(slot).cloned()
        }
        fn write(&self, slot: &str, binding: &Binding) {
            self.held
                .borrow_mut()
                .insert(slot.to_string(), binding.clone());
        }
        fn erase(&self, slot: &str) {
            self.held.borrow_mut().remove(slot);
        }
        fn set_slots(&self, slots: &[String]) {
            *self.slots.borrow_mut() = slots.to_vec();
        }
    }

    fn switch(name: &str) -> Action {
        Action::Switch(name.to_string())
    }

    #[test]
    fn a_name_slugs_the_way_the_script_slugs_it() {
        assert_eq!(slug("Work"), "work");
        assert_eq!(slug("My Work"), "my-work");
        assert_eq!(slug("Work  —  Home"), "work-home");
        assert_eq!(slug("-leading and trailing-"), "leading-and-trailing");
        assert_eq!(slug("Dock 2"), "dock-2");
    }

    /// The script's slug is byte-identical to this, and the two write the same
    /// list. Pinning the paths means a change to either side fails here rather
    /// than quietly binding one key to one action twice.
    #[test]
    fn the_slots_are_the_ones_the_script_writes() {
        assert_eq!(
            slot(&Action::Cycle),
            "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/doca-cycle/"
        );
        assert_eq!(
            slot(&switch("Work")),
            "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings/doca-work/"
        );
    }

    #[test]
    fn the_commands_are_the_ones_the_script_writes() {
        assert_eq!(
            command(&Action::Cycle),
            "gdbus call --session --dest io.github.gusmartins499.Doca \
             --object-path /io/github/gusmartins499/Doca \
             --method io.github.gusmartins499.Doca1.CycleEnvironment"
        );
        assert_eq!(
            command(&switch("My Work")),
            "gdbus call --session --dest io.github.gusmartins499.Doca \
             --object-path /io/github/gusmartins499/Doca \
             --method io.github.gusmartins499.Doca1.SetEnvironment \"My Work\""
        );
        assert_eq!(label(&Action::Cycle), "Doca: cycle environment");
        assert_eq!(label(&switch("My Work")), "Doca: My Work");
    }

    #[test]
    fn binding_keeps_every_other_application_s_keys() {
        let desktop = Fake::with(&[
            ("/custom0/", "<Super>t", "Terminal"),
            ("/custom1/", "<Print>", "Screenshot"),
        ]);

        bind(&desktop, &Action::Cycle, "<Super>e");

        let slots = desktop.slots();
        assert!(
            slots.contains(&"/custom0/".to_string())
                && slots.contains(&"/custom1/".to_string()),
            "the list is shared and came back short: {slots:?}"
        );
        assert_eq!(slots.len(), 3);
    }

    #[test]
    fn binding_the_same_action_twice_moves_its_key_rather_than_adding_a_slot() {
        let desktop = Fake::default();

        bind(&desktop, &Action::Cycle, "<Super>e");
        bind(&desktop, &Action::Cycle, "<Super>x");

        assert_eq!(desktop.slots().len(), 1, "a second slot for one action");
        assert_eq!(
            bound(&desktop, &[Action::Cycle]).get(&Action::Cycle).map(String::as_str),
            Some("<Super>x")
        );
    }

    #[test]
    fn what_was_bound_carries_the_name_and_command_the_desktop_needs() {
        let desktop = Fake::default();

        bind(&desktop, &switch("Work"), "<Super>1");

        let held = desktop.read(&slot(&switch("Work"))).expect("a binding");
        assert_eq!(held.name, "Doca: Work");
        assert_eq!(held.binding, "<Super>1");
        assert!(held.command.ends_with(".SetEnvironment \"Work\""), "{}", held.command);
    }

    #[test]
    fn unbinding_takes_one_slot_and_leaves_the_rest() {
        let desktop = Fake::with(&[("/custom0/", "<Super>t", "Terminal")]);
        bind(&desktop, &Action::Cycle, "<Super>e");
        bind(&desktop, &switch("Work"), "<Super>1");

        unbind(&desktop, &Action::Cycle);

        let actions = [Action::Cycle, switch("Work")];
        let bound = bound(&desktop, &actions);
        assert!(!bound.contains_key(&Action::Cycle), "the cycle key stayed");
        assert_eq!(bound.get(&switch("Work")).map(String::as_str), Some("<Super>1"));
        assert!(
            desktop.slots().contains(&"/custom0/".to_string()),
            "someone else's key went with it"
        );
    }

    #[test]
    fn unbinding_something_never_bound_changes_nothing() {
        let desktop = Fake::with(&[("/custom0/", "<Super>t", "Terminal")]);

        unbind(&desktop, &switch("Work"));

        assert_eq!(desktop.slots(), vec!["/custom0/".to_string()]);
    }

    #[test]
    fn an_empty_slot_is_not_a_binding_of_the_empty_key() {
        let desktop = Fake::with(&[("/custom0/", "", "Never finished")]);

        assert!(all(&desktop).is_empty());
        assert_eq!(clash(&desktop, "", &Action::Cycle, &[]), None);
    }

    #[test]
    fn a_key_another_application_holds_is_named_as_theirs() {
        let desktop = Fake::with(&[("/custom0/", "<Super>e", "Open terminal")]);

        assert_eq!(
            clash(&desktop, "<Super>e", &Action::Cycle, &[Action::Cycle]),
            Some(Clash::Theirs("Open terminal".to_string()))
        );
    }

    #[test]
    fn a_key_another_dock_holds_is_named_as_that_dock() {
        let desktop = Fake::default();
        let actions = [Action::Cycle, switch("Work")];
        bind(&desktop, &switch("Work"), "<Super>e");

        assert_eq!(
            clash(&desktop, "<Super>e", &Action::Cycle, &actions),
            Some(Clash::Ours(switch("Work")))
        );
    }

    /// Rebinding an action to the key it already has is not a clash with
    /// itself — otherwise pressing the same key twice would read as taken.
    #[test]
    fn an_action_does_not_clash_with_its_own_key() {
        let desktop = Fake::default();
        bind(&desktop, &Action::Cycle, "<Super>e");

        assert_eq!(
            clash(&desktop, "<Super>e", &Action::Cycle, &[Action::Cycle]),
            None
        );
    }

    #[test]
    fn a_free_key_clashes_with_nothing() {
        let desktop = Fake::with(&[("/custom0/", "<Super>t", "Terminal")]);

        assert_eq!(clash(&desktop, "<Super>e", &Action::Cycle, &[]), None);
    }

    #[test]
    fn the_cycle_can_always_be_bound() {
        assert_eq!(bindable(&Action::Cycle, &[]), Ok(()));
    }

    #[test]
    fn two_docks_that_slug_alike_cannot_be_bound_until_one_is_renamed() {
        let docks = ["My Work".to_string(), "My-Work".to_string()];

        let refused = bindable(&switch("My Work"), &docks).expect_err("one slot, two docks");

        assert!(refused.contains("My-Work"), "the twin is not named: {refused}");
        assert!(refused.contains("rename"), "no way out offered: {refused}");
    }

    #[test]
    fn a_dock_named_only_in_punctuation_cannot_be_bound() {
        let docks = ["···".to_string()];

        assert!(bindable(&switch("···"), &docks).is_err());
    }

    #[test]
    fn a_dock_whose_name_is_merely_unusual_can_still_be_bound() {
        let docks = ["Trabalho é bom".to_string(), "Work".to_string()];

        assert_eq!(bindable(&switch("Trabalho é bom"), &docks), Ok(()));
        assert_eq!(slug("Trabalho é bom"), "trabalho-bom");
    }
}
