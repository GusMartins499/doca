use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use zbus::zvariant::{OwnedValue, Type, Value};

pub const BUS_NAME: &str = "io.github.gusmartins499.Doca";
pub const OBJECT_PATH: &str = "/io/github/gusmartins499/Doca";
pub const INTERFACE: &str = "io.github.gusmartins499.Doca1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct WindowInfo {
    pub id: u32,
    pub title: String,
    pub app_id: String,
    pub workspace: i32,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct DockItem {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub pinned: bool,
    pub windows: Vec<u32>,
    pub active: bool,
}

/// One dock as the daemon holds it: everything a window can change about it.
///
/// `pinned` and `widgets` are here for the same reason every `Appearance`
/// field is readable: a window that can reorder pins but not read them back
/// would have to keep its own guess of the order beside the daemon's, and the
/// two would disagree the first time something else wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct EnvironmentInfo {
    pub name: String,
    pub workspaces: Vec<i32>,
    pub current: bool,
    pub pinned: Vec<String>,
    pub widgets: Vec<String>,
}

/// An installed application, as something offering a list of them needs it.
///
/// Not a `DockItem`: an app nobody has pinned and nobody is running has no
/// windows, no pin and no focus, and three fields saying so would invite a
/// caller to believe them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct Application {
    pub id: String,
    pub name: String,
    pub icon: String,
}

/// What one widget is showing, as that widget alone shapes it.
///
/// `id` sits outside the body because everything that *handles* a state
/// handles it by id and nothing else: [`WIDGETS`] is still the one list of
/// them, `ListWidgets` filters by it, and the bar's shelf reconciles its tiles
/// by it. Only the drawing needs to know which shape the body takes.
///
/// # Why a name beside a variant, and not a Rust enum on the wire
///
/// D-Bus has no marked union. `zvariant` derives [`Type`] for an enum whose
/// variants carry nothing — it writes a `u32` — and cannot derive one for an
/// enum that carries data, because there is no single signature to write for
/// "one of these five structs". That leaves two ways to spell it: a struct
/// holding every variant's fields as optionals, or `(s v)` — the variant's
/// name beside its payload as a `Variant`, which is how a union has always
/// been spelled on this bus.
///
/// The optional-fields struct is the one that lies. Its signature would say a
/// music state carries a water goal; every reader would have to decide what
/// an absent field means; and every variant added would widen the type for
/// all the others. `(s v)` says exactly what it carries: a name, and a value
/// whose own signature travels with it.
///
/// So that is what crosses the bus — `(s(sv))` with the id in front — and the
/// lie is kept out of the Rust side too. `kind` and `body` are private, the
/// only way to make one of these is [`WidgetState::new`] from a [`Body`], and
/// the only way to read one back is [`WidgetState::body`]. A name and a
/// payload that disagree is therefore not a state anyone can build.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct WidgetState {
    pub id: String,
    kind: String,
    body: OwnedValue,
}

/// What a widget is showing, in the shape belonging to that widget.
///
/// One arm per kind of widget, and the bar has one drawer per arm. Adding a
/// kind is an arm here, a name in [`body_kind`], and a drawer in the bar —
/// and nothing at all in the eleven widgets that do not take it.
#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    /// Two lines and a bar: the tile every widget drew before there were
    /// variants, and the one the widgets that have no variant of their own
    /// still draw.
    ///
    /// Not a leftover. Nine of the twelve widgets have nothing to say that
    /// this does not already say — a clock is a time and a date — and giving
    /// each of them a variant of its own would be twelve drawers to say one
    /// thing. The three that are more than this get their own as their own
    /// issues land.
    Simple(Simple),
    Water(Water),
    Music(Music),
    Note(Note),
}

/// The name each [`Body`] arm travels under, which is the `s` of the `(s v)`.
///
/// Spelled out here rather than taken from the arm's Rust name so that
/// renaming the arm cannot silently change the wire.
pub mod body_kind {
    pub const SIMPLE: &str = "simple";
    pub const WATER: &str = "water";
    pub const MUSIC: &str = "music";
    pub const NOTE: &str = "note";

    /// Every name there is, for a test to walk.
    pub const ALL: [&str; 4] = [SIMPLE, WATER, MUSIC, NOTE];
}

/// Two lines and a progress bar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, Value, OwnedValue)]
pub struct Simple {
    pub label: String,
    pub detail: String,
    /// [`NO_PROGRESS`] for a widget with nothing to measure.
    pub progress: f64,
    pub active: bool,
}

/// Glasses drunk today, against the goal that was set.
///
/// Counted in glasses because that is what the widget has always counted and
/// what `water.goal` is written in. Everything else a water tile shows — the
/// share of the goal, whether the day is done — follows from these two, so
/// neither is sent: a field that can be derived is a field that can disagree
/// with what it was derived from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, Value, OwnedValue)]
pub struct Water {
    /// Millilitres drunk today.
    pub drunk: u32,
    /// Millilitres the day is aiming at.
    pub goal: u32,
    /// How much one go is worth — the bottle the user drinks from.
    ///
    /// Here rather than only in the settings because the panel's button says
    /// what it will add, and a panel that had to ask the daemon for that
    /// would be a D-Bus call on the way to drawing a label.
    pub bottle: u32,
    /// Millilitres on each of the last seven days, oldest first, today last.
    ///
    /// Seven numbers and not seven dates: the bar draws them in order against
    /// the goal, and a date it would only use to put them in the order they
    /// already arrive in is a field free to disagree with that order.
    pub week: Vec<u32>,
}

/// What is playing, and the picture that goes with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, Value, OwnedValue)]
pub struct Music {
    pub title: String,
    pub artist: String,
    /// Who is playing it, for when the track has no name of its own.
    pub player: String,
    pub playing: bool,
    /// The cover, as a path to a file on this machine. Empty for a track with
    /// no art, or art this build could not get hold of.
    ///
    /// # Why a path and not the bytes
    ///
    /// The other way to spell this is the image itself on the signal, which
    /// assumes nothing about who can see what. It is the wrong trade here for
    /// two reasons, and the second is the one that settles it.
    ///
    /// MPRIS sends metadata on every change, several times a track on some
    /// players, so bytes on the bus is a picture on the bus over and over for
    /// a picture that did not change.
    ///
    /// And the assumption a path makes — that the daemon and the bar see the
    /// same disk — is one this contract already makes everywhere else.
    /// [`DockItem::icon`] is a path or a theme name the *bar* loads, and
    /// [`FolderEntry::path`] is a file the bar opens. A cover sent as bytes
    /// would be the only thing here that did not trust the filesystem both
    /// ends are already standing on.
    pub art: String,
}

/// A note, whole, and the colour of the paper it is written on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type, Value, OwnedValue)]
pub struct Note {
    /// The note as it was written, newlines and all.
    ///
    /// Whole rather than split into a first line and a rest, which is what
    /// the daemon used to send. The tile shows what fits and the panel edits
    /// the lot, and both of those need the text as it is — a split made in
    /// the daemon is a guess about how much room the bar has.
    pub text: String,
    /// One of [`note_colour::ALL`].
    pub colour: String,
}

/// The papers a note can be written on.
///
/// The primo.dock's six, kept whole rather than pruned: a post-it colour is
/// not a setting anybody needs explained, it costs a line each, and the one
/// somebody wants is always the one that was cut.
pub mod note_colour {
    pub const YELLOW: &str = "yellow";
    pub const PINK: &str = "pink";
    pub const BLUE: &str = "blue";
    pub const GREEN: &str = "green";
    pub const PURPLE: &str = "purple";
    pub const RED: &str = "red";

    pub const ALL: [&str; 6] = [YELLOW, PINK, BLUE, GREEN, PURPLE, RED];

    /// The one a config that never asked gets.
    pub const DEFAULT: &str = YELLOW;

    pub fn resolve(asked: &str) -> &'static str {
        ALL.into_iter().find(|name| *name == asked).unwrap_or(DEFAULT)
    }
}

/// How much room a tile asks for on the bar: about one icon, or about two
/// and a half.
///
/// Declared by the [`Body`] and not carried in it. A width on the wire would
/// be a second truth sitting beside the variant that already implies it, free
/// to disagree with it; the variant is the declaration, and this is read off
/// it at both ends by the same function.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tile {
    Square,
    Wide,
}

impl Tile {
    /// How many icons wide each shape is.
    ///
    /// The bar sizes its icons to fit the screen and the tiles to the icon
    /// size the config asked for, so these are the whole of how wide a tile
    /// is — which is what lets the bar reserve the room for its tiles before
    /// it has worked out how big its icons may be.
    pub const SQUARE_ICONS: f64 = 1.0;
    pub const WIDE_ICONS: f64 = 2.5;

    pub fn icons(self) -> f64 {
        match self {
            Tile::Square => Self::SQUARE_ICONS,
            Tile::Wide => Self::WIDE_ICONS,
        }
    }

    /// The tile's own width at this icon size, before whatever frame the bar
    /// draws around it.
    pub fn width(self, icon_size: i32) -> i32 {
        (icon_size as f64 * self.icons()).round() as i32
    }
}

impl Body {
    pub fn kind(&self) -> &'static str {
        match self {
            Body::Simple(_) => body_kind::SIMPLE,
            Body::Water(_) => body_kind::WATER,
            Body::Music(_) => body_kind::MUSIC,
            Body::Note(_) => body_kind::NOTE,
        }
    }

    /// How wide a tile this body is drawn in.
    ///
    /// `Simple` is wide because it is the tile the nine widgets already drew
    /// — two lines of text and a bar under them do not fit in a square, and
    /// the acceptance for those nine is that they draw what they drew before.
    /// `Water` is square because the chassis draws it as the count against
    /// its goal and nothing else; the wide card with the week in it is #28's,
    /// and this is one line to change when it lands.
    pub fn tile(&self) -> Tile {
        match self {
            Body::Simple(_) => Tile::Wide,
            // A bottle filling needs height to read as filling, and the
            // amount beside it needs room to be read at a glance: a badge one
            // icon wide could hold one of the two.
            Body::Water(_) => Tile::Wide,
            // A cover is a picture, and a picture one icon wide is a thumbnail
            // with the title written over it.
            Body::Music(_) => Tile::Wide,
            // A post-it is a square. It is the one tile whose shape is the
            // thing it is standing for rather than a choice about room.
            Body::Note(_) => Tile::Square,
        }
    }
}

/// A body whose payload is not the shape its name claims.
///
/// Only reachable from a reader that is older or newer than the writer: a
/// `kind` nobody here knows, or a payload that will not read back as the
/// struct that name stands for. Both are things a bar has to survive rather
/// than crash on, so they come back as a sentence it can log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnreadableBody {
    pub kind: String,
    pub why: String,
}

impl std::fmt::Display for UnreadableBody {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "a {} body did not read back: {}", self.kind, self.why)
    }
}

impl std::error::Error for UnreadableBody {}

impl WidgetState {
    /// The only way to build one, which is why the name and the payload
    /// cannot drift apart.
    ///
    /// Infallible, though the conversion under it is not: turning a value
    /// into an owned one can only fail for a file descriptor, and no body
    /// carries one. `every_variant_survives_the_wire` holds that for each
    /// arm, so an arm that ever did would fail the suite rather than the
    /// daemon.
    pub fn new(id: impl Into<String>, body: Body) -> Self {
        let kind = body.kind().to_string();
        let value = match body {
            Body::Simple(simple) => OwnedValue::try_from(simple),
            Body::Water(water) => OwnedValue::try_from(water),
            Body::Music(music) => OwnedValue::try_from(music),
            Body::Note(note) => OwnedValue::try_from(note),
        };
        Self {
            id: id.into(),
            kind,
            body: value.expect("a widget body holds no file descriptor"),
        }
    }

    /// Which arm this is, without reading the payload.
    pub fn kind(&self) -> &str {
        &self.kind
    }

    /// The payload, matched against the name it travelled under.
    ///
    /// The one place that match happens, so a reader never has to know how
    /// the union is spelled.
    pub fn body(&self) -> Result<Body, UnreadableBody> {
        let unreadable = |why: String| UnreadableBody {
            kind: self.kind.clone(),
            why,
        };
        match self.kind.as_str() {
            body_kind::SIMPLE => Simple::try_from(self.body.clone())
                .map(Body::Simple)
                .map_err(|e| unreadable(e.to_string())),
            body_kind::WATER => Water::try_from(self.body.clone())
                .map(Body::Water)
                .map_err(|e| unreadable(e.to_string())),
            body_kind::MUSIC => Music::try_from(self.body.clone())
                .map(Body::Music)
                .map_err(|e| unreadable(e.to_string())),
            body_kind::NOTE => Note::try_from(self.body.clone())
                .map(Body::Note)
                .map_err(|e| unreadable(e.to_string())),
            unknown => Err(UnreadableBody {
                kind: unknown.to_string(),
                why: format!("no such body; this build knows {}", body_kind::ALL.join(", ")),
            }),
        }
    }

    /// How wide a tile to draw, or `None` for a body this build cannot read.
    pub fn tile(&self) -> Option<Tile> {
        self.body().ok().map(|body| body.tile())
    }
}

pub const NO_PROGRESS: f64 = -1.0;

/// The look of the dock as the daemon reports it.
///
/// Every field a writer can set is a field a reader can see: a preferences
/// window that could turn the trash on but not read back whether it is on
/// would have to keep its own guess of the truth beside the daemon's.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
pub struct Appearance {
    pub theme: String,
    pub icon_size: i32,
    pub magnification: f64,
    pub auto_hide: bool,
    pub show_trash: bool,
    /// An icon, GTK or cursor theme for the dock alone. Empty follows the
    /// system, which is what it does unless someone says otherwise.
    pub icon_theme: String,
    pub gtk_theme: String,
    pub cursor_theme: String,
}

/// The settings the widgets take, as the daemon reports them.
///
/// Flat, and named `widget_key` by `widget_key`, because that is the shape
/// `SetWidgetSetting` writes in: a getter grouped differently from the setter
/// is two spellings of the same five values to keep in step. Every field here
/// is a field that can be written, for the reason [`Appearance`] gives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct WidgetSettings {
    pub countdown_date: String,
    pub countdown_label: String,
    pub note_text: String,
    pub timer_minutes: u32,
    /// Millilitres.
    pub water_goal: u32,
    /// Millilitres in one bottle.
    pub water_bottle: u32,
    /// One of [`note_colour::ALL`].
    pub note_colour: String,
}

/// The widgets that take a setting, and what `SetWidgetSetting` calls each.
///
/// A window building the controls needs the exact pair the daemon validates
/// against; the daemon's own `match` is the other half of this and `docad`
/// has a test that no pair here is refused.
pub mod widget_key {
    pub const COUNTDOWN: &str = "countdown";
    pub const NOTE: &str = "note";
    pub const TIMER: &str = "timer";
    pub const WATER: &str = "water";

    pub const DATE: &str = "date";
    pub const LABEL: &str = "label";
    pub const TEXT: &str = "text";
    pub const MINUTES: &str = "minutes";
    pub const GOAL: &str = "goal";
    pub const BOTTLE: &str = "bottle";
    pub const COLOUR: &str = "colour";

    /// Every (widget, key) pair that exists, for a test to walk.
    pub const ALL: [(&str, &str); 7] = [
        (COUNTDOWN, DATE),
        (COUNTDOWN, LABEL),
        (NOTE, TEXT),
        (NOTE, COLOUR),
        (TIMER, MINUTES),
        (WATER, GOAL),
        (WATER, BOTTLE),
    ];
}

/// What each widget can be asked to do, and what to call it on a control.
///
/// Here for the reason [`widget_key`] is: the daemon's own `match` is the
/// other half of it, and a panel offering these has to work from the pairs
/// the daemon really accepts rather than from a list written out by hand.
/// `docad` has a test that every action here is one that widget answers to,
/// so an action renamed on one side alone fails the suite instead of
/// becoming a button that does nothing.
///
/// Not every action a widget accepts is here — a widget that takes both
/// `start` and `pause` is offered one `toggle` — because this is the list of
/// controls to *draw*, not the list of words `InvokeWidget` will take. The
/// widgets that are absent altogether have nothing to be asked: a clock
/// cannot be told anything about the time.
pub mod widget_action {
    /// Every (widget, action, label) there is, in the order a panel should
    /// offer them.
    pub const ALL: [(&str, &str, &str); 17] = [
        ("music", "toggle", "Play or pause"),
        ("music", "previous", "Previous track"),
        ("music", "next", "Next track"),
        ("pomodoro", "toggle", "Start or pause"),
        ("pomodoro", "skip", "Skip this block"),
        ("pomodoro", "reset", "Reset"),
        ("stopwatch", "toggle", "Start or stop"),
        ("stopwatch", "lap", "Lap"),
        ("stopwatch", "reset", "Reset"),
        ("timer", "toggle", "Start or pause"),
        ("timer", "add", "One more minute"),
        ("timer", "reset", "Reset"),
        ("time-progress", "next", "Next span"),
        ("time-progress", "reset", "Back to today"),
        ("water", "drink", "One more bottle"),
        ("water", "undo", "Take one back"),
        ("water", "reset", "Start the day over"),
    ];

    /// What this widget offers, in order. Empty for one that offers nothing.
    pub fn of(widget: &str) -> impl Iterator<Item = (&'static str, &'static str)> + '_ {
        ALL.into_iter()
            .filter(move |(id, _, _)| *id == widget)
            .map(|(_, action, label)| (action, label))
    }
}

/// A date as the countdown widget reads it: `YYYY-MM-DD` and nothing else.
///
/// Here rather than in the daemon because both ends need the same answer: the
/// widget turns the text into a day, and a window has to be able to say "that
/// is not a date" before sending text that would show up in the bar as
/// "bad date" with no explanation of why.
pub fn parse_date(value: &str) -> Option<(i64, u32, u32)> {
    let mut parts = value.trim().split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    Some((year, month, day))
}

/// The themes the bar can wear, in the order something listing them should.
///
/// Here rather than in the shell because three crates need to agree on it: the
/// shell owns the stylesheets, the daemon validates what is asked for, and a
/// preferences window has to offer the list without writing it out by hand —
/// a hand-written copy is a list that goes stale the first time a theme is
/// added.
pub const THEMES: [&str; 4] = ["system", "native", "midnight", "paper"];

/// The widgets a dock can show, in the order something listing them should.
///
/// Here for the reason [`THEMES`] is: the daemon builds them, and a window has
/// to offer the list without writing it out by hand. `docad` has a test that
/// every id here builds into a real widget, so a name that goes stale fails
/// the suite rather than logging "ignoring unknown widget" at someone.
pub const WIDGETS: [&str; 12] = [
    "clock",
    "battery",
    "cpu",
    "network",
    "music",
    "time-progress",
    "pomodoro",
    "timer",
    "stopwatch",
    "countdown",
    "water",
    "note",
];

/// The theme a config that never mentioned one gets.
pub const DEFAULT_THEME: &str = "native";

/// The limits every writer is held to, so a window can show them as a range
/// instead of guessing and being corrected after the fact.
pub const MIN_ICON_SIZE: i32 = 24;
pub const MAX_ICON_SIZE: i32 = 96;
/// 1.0 is how the lens is turned off, so it is also the floor.
pub const MIN_MAGNIFICATION: f64 = 1.0;
pub const MAX_MAGNIFICATION: f64 = 2.5;
/// A timer of no minutes has nothing to count, so one is the floor.
pub const MIN_TIMER_MINUTES: u32 = 1;
pub const MAX_TIMER_MINUTES: u32 = 24 * 60;
/// Millilitres, not glasses.
///
/// A glass is a unit nobody's bottle is marked in. The range is the one
/// primo.dock offers; a config written when this counted glasses is carried
/// over by [`crate::GLASS`] rather than clamped, and the two ranges do not
/// overlap, which is what makes that migration safe to run more than once.
pub const MIN_WATER_GOAL: u32 = 500;
pub const MAX_WATER_GOAL: u32 = 6000;
/// What a glass was worth, for carrying an old config over.
///
/// Eight of them is 2000ml, which is both the old default and the new one —
/// so somebody who never changed the setting does not notice it changed
/// units.
pub const GLASS: u32 = 250;
pub const MIN_WATER_BOTTLE: u32 = 100;
pub const MAX_WATER_BOTTLE: u32 = 2000;

/// The keys `SetAppearance` understands, by the name they carry on the wire.
pub mod appearance_key {
    pub const THEME: &str = "theme";
    pub const ICON_SIZE: &str = "icon_size";
    pub const MAGNIFICATION: &str = "magnification";
    pub const AUTO_HIDE: &str = "auto_hide";
    pub const SHOW_TRASH: &str = "show_trash";
    pub const ICON_THEME: &str = "icon_theme";
    pub const GTK_THEME: &str = "gtk_theme";
    pub const CURSOR_THEME: &str = "cursor_theme";

    pub const ALL: [&str; 8] = [
        THEME,
        ICON_SIZE,
        MAGNIFICATION,
        AUTO_HIDE,
        SHOW_TRASH,
        ICON_THEME,
        GTK_THEME,
        CURSOR_THEME,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Type)]
pub struct FolderEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
}

#[zbus::proxy(
    interface = "io.github.gusmartins499.Doca1",
    default_service = "io.github.gusmartins499.Doca",
    default_path = "/io/github/gusmartins499/Doca"
)]
pub trait Doca {
    fn list_environments(&self) -> zbus::Result<Vec<EnvironmentInfo>>;
    fn current_environment(&self) -> zbus::Result<String>;
    fn set_environment(&self, name: &str) -> zbus::Result<()>;
    fn cycle_environment(&self) -> zbus::Result<String>;

    fn appearance(&self) -> zbus::Result<Appearance>;

    /// Change only the keys named, leaving the rest of the look alone.
    ///
    /// A dictionary rather than a whole `Appearance` so that a window with one
    /// slider under the pointer does not have to send back five values it read
    /// a minute ago — and so adding a key later does not break a caller that
    /// never heard of it.
    fn set_appearance(&self, changes: HashMap<String, OwnedValue>) -> zbus::Result<()>;

    /// What every widget setting is right now.
    ///
    /// The counterpart of `SetWidgetSetting`, and already clamped: a window
    /// can put these straight on its controls without checking them again.
    fn widget_settings(&self) -> zbus::Result<WidgetSettings>;

    fn set_widget_setting(
        &self,
        widget: &str,
        key: &str,
        value: OwnedValue,
    ) -> zbus::Result<()>;

    fn set_environment_widgets(&self, name: &str, widgets: Vec<String>) -> zbus::Result<()>;
    fn set_environment_workspaces(&self, name: &str, workspaces: Vec<i32>) -> zbus::Result<()>;
    fn reorder_pinned(&self, name: &str, order: Vec<String>) -> zbus::Result<()>;

    /// Put the docks in a new order, which is the order the cycle walks.
    ///
    /// Only an order: a name nobody knows is refused, and a dock left out
    /// keeps its place at the end — so a window holding a list drawn before a
    /// rename reorders what it knows instead of deleting what it does not.
    fn reorder_environments(&self, order: Vec<String>) -> zbus::Result<()>;
    fn add_environment(&self, name: &str) -> zbus::Result<String>;
    fn remove_environment(&self, name: &str) -> zbus::Result<()>;
    fn rename_environment(&self, from: &str, to: &str) -> zbus::Result<String>;

    /// Every installed application, for a window that has to offer a choice
    /// of them. Sorted by name, and apps that ask not to be shown are not.
    fn list_applications(&self) -> zbus::Result<Vec<Application>>;

    /// Pin to a named dock rather than the one on screen.
    ///
    /// `PinItem` pins where the user is looking, which is what a click on the
    /// bar means. A preferences window is editing a dock it may not be
    /// standing in, so it has to name the one it means.
    fn pin_in(&self, name: &str, id: &str) -> zbus::Result<()>;
    fn unpin_in(&self, name: &str, id: &str) -> zbus::Result<()>;

    fn list_widgets(&self) -> zbus::Result<Vec<WidgetState>>;
    fn invoke_widget(&self, id: &str, action: &str) -> zbus::Result<()>;

    fn list_items(&self) -> zbus::Result<Vec<DockItem>>;
    fn activate_item(&self, id: &str) -> zbus::Result<()>;
    fn launch_item(&self, id: &str) -> zbus::Result<()>;
    fn open_with(&self, id: &str, paths: &[&str]) -> zbus::Result<()>;
    fn list_folder(&self, id: &str) -> zbus::Result<Vec<FolderEntry>>;
    fn open_path(&self, path: &str) -> zbus::Result<()>;
    fn item_windows(&self, id: &str) -> zbus::Result<Vec<WindowInfo>>;
    fn pin_item(&self, id: &str) -> zbus::Result<()>;
    fn unpin_item(&self, id: &str) -> zbus::Result<()>;
    fn close_window(&self, id: u32) -> zbus::Result<()>;

    fn list_windows(&self) -> zbus::Result<Vec<WindowInfo>>;
    fn activate_window(&self, id: u32) -> zbus::Result<()>;
    fn current_workspace(&self) -> zbus::Result<i32>;
    fn workspace_count(&self) -> zbus::Result<i32>;
    fn set_workspace(&self, index: i32) -> zbus::Result<()>;

    /// The config on disk changed, whoever changed it.
    ///
    /// One signal for the whole file rather than one per key: everything a
    /// reader does with it ends in the same re-read, and a bar that rebuilds
    /// once is cheaper than a bar that rebuilds five times because five keys
    /// moved together.
    #[zbus(signal)]
    fn config_changed(&self) -> zbus::Result<()>;

    #[zbus(signal)]
    fn environment_changed(&self, name: &str) -> zbus::Result<()>;

    #[zbus(signal)]
    fn widget_changed(&self, state: WidgetState) -> zbus::Result<()>;

    #[zbus(signal)]
    fn items_changed(&self) -> zbus::Result<()>;

    #[zbus(signal)]
    fn windows_changed(&self) -> zbus::Result<()>;

    #[zbus(signal)]
    fn workspace_changed(&self, index: i32) -> zbus::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_date_string_parses_only_when_it_is_really_a_date() {
        assert_eq!(parse_date("2026-12-25"), Some((2026, 12, 25)));
        assert_eq!(parse_date(" 2026-01-01 "), Some((2026, 1, 1)));
        assert_eq!(parse_date("2026-13-01"), None);
        assert_eq!(parse_date("2026-00-10"), None);
        assert_eq!(parse_date("25/12/2026"), None);
        assert_eq!(parse_date("2026-12-25-01"), None);
        assert_eq!(parse_date("tomorrow"), None);
        assert_eq!(parse_date(""), None);
    }

    /// Both lists are walked by something; neither may hold a name twice.
    #[test]
    fn nothing_is_offered_twice() {
        let mut widgets = WIDGETS.to_vec();
        widgets.sort_unstable();
        let was = widgets.len();
        widgets.dedup();
        assert_eq!(widgets.len(), was, "a widget id is in the list twice");

        let mut pairs = widget_key::ALL.to_vec();
        pairs.sort_unstable();
        let was = pairs.len();
        pairs.dedup();
        assert_eq!(pairs.len(), was, "a widget setting is in the list twice");
    }

    /// Every widget a setting is addressed to has to be a widget on offer.
    #[test]
    fn a_setting_belongs_to_a_widget_that_exists() {
        for (widget, key) in widget_key::ALL {
            assert!(
                WIDGETS.contains(&widget),
                "{widget}.{key} names a widget that is not on offer"
            );
        }
    }

    /// The same for the actions a panel draws buttons from.
    #[test]
    fn an_action_belongs_to_a_widget_that_exists() {
        for (widget, action, label) in widget_action::ALL {
            assert!(
                WIDGETS.contains(&widget),
                "{widget}.{action} names a widget that is not on offer"
            );
            assert!(!label.trim().is_empty(), "{widget}.{action} has no label to draw");
        }
    }

    #[test]
    fn no_widget_offers_the_same_action_twice() {
        let mut pairs: Vec<(&str, &str)> = widget_action::ALL
            .iter()
            .map(|(widget, action, _)| (*widget, *action))
            .collect();
        pairs.sort_unstable();
        let was = pairs.len();
        pairs.dedup();
        assert_eq!(was, pairs.len(), "an action is offered twice");
    }

    #[test]
    fn the_actions_of_one_widget_are_the_ones_belonging_to_it() {
        let water: Vec<&str> = widget_action::of("water").map(|(a, _)| a).collect();

        assert_eq!(water, vec!["drink", "undo", "reset"]);
        assert_eq!(widget_action::of("clock").count(), 0, "a clock takes no orders");
    }

    fn simple() -> Body {
        Body::Simple(Simple {
            label: "17:21".to_string(),
            detail: "Thu 01/01".to_string(),
            progress: NO_PROGRESS,
            active: false,
        })
    }

    fn every_body() -> Vec<(&'static str, Body)> {
        vec![
            (body_kind::SIMPLE, simple()),
            (
                body_kind::WATER,
                Body::Water(Water { drunk: 3, goal: 8, bottle: 500, week: vec![0; 7] }),
            ),
            (
                body_kind::NOTE,
                Body::Note(Note {
                    text: "milk\nand bread".to_string(),
                    colour: note_colour::PINK.to_string(),
                }),
            ),
            (
                body_kind::MUSIC,
                Body::Music(Music {
                    title: "Girl from Ipanema".to_string(),
                    artist: "João Gilberto".to_string(),
                    player: "rhythmbox".to_string(),
                    playing: true,
                    art: "/tmp/cover.png".to_string(),
                }),
            ),
        ]
    }

    /// The whole of the union: what went in is what comes back, through the
    /// very bytes the bus would carry.
    ///
    /// The encoding is the part worth checking rather than the struct. A
    /// `(s v)` whose payload is written one way and read another is a bar
    /// that comes up with no tiles on it and nothing in the log to say why.
    #[test]
    fn every_variant_survives_the_wire() {
        use zbus::zvariant::{serialized::Context, to_bytes, Endian};

        for (kind, body) in every_body() {
            let state = WidgetState::new("probe", body.clone());
            assert_eq!(state.kind(), kind);

            let context = Context::new_dbus(Endian::Little, 0);
            let bytes = to_bytes(context, &state).expect("a state serialises");
            let (back, _): (WidgetState, _) =
                bytes.deserialize().expect("a state deserialises");

            assert_eq!(back.id, "probe");
            assert_eq!(back.kind(), kind);
            assert_eq!(back.body().expect("the body reads back"), body);
            assert_eq!(back, state, "a state that crossed the bus is a different state");
        }
    }

    /// Every arm has a name, and every name has an arm.
    #[test]
    fn no_variant_travels_under_a_name_nothing_knows() {
        let named: Vec<&str> = every_body().iter().map(|(kind, _)| *kind).collect();

        for kind in body_kind::ALL {
            assert!(named.contains(&kind), "{kind} is a name with no body behind it");
        }
        assert_eq!(named.len(), body_kind::ALL.len());
    }

    /// A reader older than the writer has to say so rather than fall over.
    #[test]
    fn a_body_this_build_never_heard_of_is_a_sentence_and_not_a_panic() {
        let state = WidgetState::new("probe", simple());
        let forged = WidgetState {
            id: state.id.clone(),
            kind: "calendar".to_string(),
            body: state.body.clone(),
        };

        let failed = forged.body().expect_err("an unknown body cannot be read");

        assert_eq!(failed.kind, "calendar");
        assert!(
            failed.to_string().contains("calendar"),
            "the complaint does not name the body: {failed}"
        );
        assert_eq!(forged.tile(), None, "an unreadable body was given a size anyway");
    }

    /// And a payload that is not the shape its name claims.
    #[test]
    fn a_payload_that_does_not_match_its_name_is_refused() {
        let water = WidgetState::new("water", Body::Water(Water { drunk: 1, goal: 8, bottle: 500, week: vec![0; 7] }));
        let lying = WidgetState {
            id: water.id.clone(),
            kind: body_kind::SIMPLE.to_string(),
            body: water.body.clone(),
        };

        assert!(lying.body().is_err(), "two integers read back as a Simple");
    }

    /// Which shape each variant asks for is a contract fact, not a drawing
    /// detail: the bar reserves the room before it has drawn anything, and a
    /// variant that changed its mind quietly would be a bar that is the wrong
    /// width on the frame the widget arrives.
    #[test]
    fn every_variant_declares_the_shape_the_bar_reserves_for_it() {
        assert_eq!(
            Body::Simple(Simple {
                label: String::new(),
                detail: String::new(),
                progress: NO_PROGRESS,
                active: false,
            })
            .tile(),
            Tile::Wide
        );
        // Water is wide: a bottle filling needs height to read as filling and
        // the amount beside it needs room to be read at a glance, and a badge
        // one icon wide holds one of the two.
        assert_eq!(
            Body::Water(Water { drunk: 0, goal: 2000, bottle: 500, week: vec![0; 7] }).tile(),
            Tile::Wide
        );
    }

    #[test]
    fn a_wide_tile_is_two_and_a_half_icons_and_a_square_one_is_one() {
        assert_eq!(Tile::Square.width(48), 48);
        assert_eq!(Tile::Wide.width(48), 120);
        assert_eq!(Tile::Square.width(24), 24);
        assert_eq!(Tile::Wide.width(96), 240);
    }

    /// The acceptance that a tile does not stay the same size when the icons
    /// go from 24 to 96.
    #[test]
    fn a_tile_grows_with_the_icons_it_stands_beside() {
        for tile in [Tile::Square, Tile::Wide] {
            assert!(
                tile.width(MAX_ICON_SIZE) > tile.width(MIN_ICON_SIZE),
                "a {tile:?} tile is the same width at 24px icons as at 96px"
            );
        }
        assert!(
            Tile::Wide.width(MIN_ICON_SIZE) > Tile::Square.width(MIN_ICON_SIZE),
            "wide is not wider than square"
        );
    }

    /// The nine widgets that kept their tile kept the four things it showed.
    #[test]
    fn a_simple_body_still_carries_exactly_what_the_old_tile_drew() {
        let Body::Simple(simple) = simple() else {
            panic!("not a simple body");
        };

        assert_eq!(simple.label, "17:21");
        assert_eq!(simple.detail, "Thu 01/01");
        assert_eq!(simple.progress, NO_PROGRESS);
        assert!(!simple.active);
    }
}
