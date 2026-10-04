use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gtk::prelude::*;
use doca_ipc::{WidgetState, NO_PROGRESS};

pub const TILE_WIDTH: i32 = 88;

/// What a click on a tile asks the daemon to do, by widget id and action.
///
/// A closure rather than the bus proxy the tile used to hold: the proxy was
/// the only reason a tile could not exist without a session bus, and the
/// shelf below is exactly the thing worth checking without one.
pub type Invoke = Rc<dyn Fn(&str, &str)>;

pub struct WidgetTile {
    pub root: gtk::Widget,
    label: gtk::Label,
    detail: gtk::Label,
    progress: gtk::ProgressBar,
}

impl WidgetTile {
    pub fn new(state: &WidgetState, invoke: Invoke) -> Self {
        let label = gtk::Label::new(None);
        label.set_widget_name("widget-label");
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_max_width_chars(11);

        let detail = gtk::Label::new(None);
        detail.set_widget_name("widget-detail");
        detail.set_ellipsize(gtk::pango::EllipsizeMode::End);
        detail.set_max_width_chars(13);

        let progress = gtk::ProgressBar::new();
        progress.set_widget_name("widget-progress");
        progress.set_valign(gtk::Align::Center);

        let column = gtk::Box::new(gtk::Orientation::Vertical, 2);
        column.set_valign(gtk::Align::Center);
        column.add(&label);
        column.add(&detail);
        column.add(&progress);

        let tile = gtk::EventBox::new();
        tile.set_widget_name("widget");
        tile.set_size_request(TILE_WIDTH, -1);
        tile.add(&column);

        let id = state.id.clone();
        tile.connect_button_press_event(move |_, event| {
            let action = match event.button() {
                3 => "reset",
                _ => "toggle",
            };
            invoke(&id, action);
            glib::Propagation::Stop
        });

        let tile = Self {
            root: tile.upcast(),
            label,
            detail,
            progress,
        };
        tile.update(state);
        tile
    }

    pub fn update(&self, state: &WidgetState) {
        self.label.set_text(&state.label);
        self.detail.set_text(&state.detail);

        if state.progress == NO_PROGRESS {
            self.progress.hide();
        } else {
            self.progress.set_fraction(state.progress.clamp(0.0, 1.0));
            self.progress.show();
        }

        let style = self.root.style_context();
        if state.active {
            style.add_class("active");
        } else {
            style.remove_class("active");
        }
    }
}

/// Every widget tile on the bar, and the divider that stands in front of them.
///
/// The bar is rebuilt whenever a window opens, closes or takes focus, and now
/// on every change to the config as well — which means on every frame of a
/// slider being dragged in the preferences window. Tearing the tiles down and
/// building them again each time is a visible flicker at that rate, and it
/// throws away the one thing a tile is: a `GtkWidget` with a style class, a
/// progress bar's shown-or-hidden state, and an X window of its own.
///
/// So this reconciles by id, the way [`crate::row::Row::fill`] does for the
/// icons. The flicker was never fixed there — it moved address, out of the
/// icons and into the widgets.
/// What one [`Shelf::show`] did: tiles reused, tiles built, tiles dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Reconciled {
    pub kept: usize,
    pub made: usize,
    pub gone: usize,
}

#[derive(Default)]
pub struct Shelf {
    /// Built once and kept, so that taking it off the bar when no widget is
    /// shown does not destroy it.
    divider: RefCell<Option<gtk::Separator>>,
    /// The tiles on the bar, in the order they are shown.
    tiles: RefCell<Vec<(String, WidgetTile)>>,
}

impl Shelf {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether this widget is one of ours, and must survive a rebuild.
    ///
    /// The rebuild sweeps the bar clear of everything it is about to make
    /// again. What the shelf owns is not made again, so it has to be asked.
    pub fn owns(&self, child: &gtk::Widget) -> bool {
        if self
            .divider
            .borrow()
            .as_ref()
            .is_some_and(|divider| divider.clone().upcast::<gtk::Widget>() == *child)
        {
            return true;
        }
        self.tiles
            .borrow()
            .iter()
            .any(|(_, tile)| tile.root == *child)
    }

    /// Put these widgets on the bar, keeping the tiles already there.
    ///
    /// A tile whose id comes back is the same tile, moved into place rather
    /// than remade. One that does not come back is taken off the bar and
    /// dropped — which is when its widget is finally destroyed, because until
    /// then this held the only other reference to it.
    ///
    /// Order is settled by moving each child to the end in turn, never by
    /// unparenting: taking a widget out of a container and putting it back is
    /// the destroy-and-rebuild this exists to avoid.
    ///
    /// Answers with what it did, because reconciliation is invisible when it
    /// works — a shelf that quietly went back to rebuilding everything looks
    /// exactly like one that did not, and the counts are the only thing that
    /// tells them apart.
    pub fn show(
        &self,
        items: &gtk::Box,
        widgets: &[WidgetState],
        invoke: &Invoke,
    ) -> Reconciled {
        let mut known: HashMap<String, WidgetTile> =
            self.tiles.borrow_mut().drain(..).collect();
        let had = known.len();

        let shown: Vec<(String, WidgetTile)> = widgets
            .iter()
            .map(|state| {
                let tile = match known.remove(&state.id) {
                    Some(tile) => {
                        tile.update(state);
                        tile
                    }
                    None => {
                        let tile = WidgetTile::new(state, invoke.clone());
                        items.add(&tile.root);
                        tile
                    }
                };
                (state.id.clone(), tile)
            })
            .collect();

        // Whatever no dock asks for any more.
        let gone = known.len();
        for (_, departed) in known {
            items.remove(&departed.root);
        }
        let did = Reconciled {
            kept: had - gone,
            made: shown.len() - (had - gone),
            gone,
        };
        // Read back by scripts/check-live.sh, which has no other way to tell
        // a bar that kept its tiles from one that made them again.
        tracing::debug!(
            kept = did.kept,
            made = did.made,
            gone = did.gone,
            "the widget shelf"
        );

        if shown.is_empty() {
            // Nothing to divide from. A separator with no widgets behind it
            // is a stray line at the end of the bar.
            if let Some(divider) = self.divider.borrow().as_ref() {
                if divider.parent().is_some() {
                    items.remove(divider);
                }
            }
        } else {
            let divider = self
                .divider
                .borrow_mut()
                .get_or_insert_with(crate::dock::divider)
                .clone();
            if divider.parent().is_none() {
                items.add(&divider);
            }
            items.reorder_child(&divider, -1);
            for (_, tile) in &shown {
                items.reorder_child(&tile.root, -1);
            }
        }

        *self.tiles.borrow_mut() = shown;
        did
    }

    /// Say again what every tile is showing.
    ///
    /// Called after the bar's `show_all`, and that order is load-bearing:
    /// `show_all` shows every child, including a progress bar that [`update`]
    /// had hidden because the widget has no progress to report. Showing it
    /// again here is what puts it back.
    ///
    /// [`update`]: WidgetTile::update
    pub fn refresh(&self, widgets: &[WidgetState]) {
        for state in widgets {
            self.update(state);
        }
    }

    /// One widget's new state, if it has a tile on the bar.
    ///
    /// `false` for a widget the bar is not showing — which is the signal that
    /// a rebuild is needed, rather than something to report as an error.
    pub fn update(&self, state: &WidgetState) -> bool {
        match self
            .tiles
            .borrow()
            .iter()
            .find(|(id, _)| *id == state.id)
        {
            Some((_, tile)) => {
                tile.update(state);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    fn state(id: &str) -> WidgetState {
        WidgetState {
            id: id.to_string(),
            label: id.to_string(),
            detail: String::new(),
            progress: NO_PROGRESS,
            active: false,
        }
    }

    /// A shelf and the bar it draws on, with something standing where the row
    /// of icons stands — the thing the shelf must never move.
    fn bar() -> (gtk::Box, gtk::Widget, Shelf, Invoke) {
        let items = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let row = gtk::Label::new(Some("the icons")).upcast::<gtk::Widget>();
        items.add(&row);
        (items, row, Shelf::new(), Rc::new(|_: &str, _: &str| {}))
    }

    fn roots(shelf: &Shelf) -> Vec<gtk::Widget> {
        shelf
            .tiles
            .borrow()
            .iter()
            .map(|(_, tile)| tile.root.clone())
            .collect()
    }

    /// The one that matters. Everything else here is about this holding under
    /// a list that moved.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn showing_the_same_widgets_again_keeps_the_very_same_tiles() {
        let (items, _, shelf, invoke) = bar();
        let first = shelf.show(&items, &[state("clock"), state("cpu")], &invoke);
        let before = roots(&shelf);

        let again = shelf.show(&items, &[state("clock"), state("cpu")], &invoke);

        assert_eq!(first, Reconciled { kept: 0, made: 2, gone: 0 });
        assert_eq!(
            again,
            Reconciled { kept: 2, made: 0, gone: 0 },
            "a rebuild that changed nothing made tiles anyway"
        );
        assert_eq!(before, roots(&shelf), "the tiles were built again");
    }

    pub fn a_widget_that_went_takes_its_tile_off_the_bar() {
        let (items, _, shelf, invoke) = bar();
        shelf.show(&items, &[state("clock"), state("cpu")], &invoke);
        let clock = roots(&shelf)[0].clone();
        let cpu = roots(&shelf)[1].clone();

        let did = shelf.show(&items, &[state("clock")], &invoke);

        assert_eq!(did, Reconciled { kept: 1, made: 0, gone: 1 });
        assert_eq!(roots(&shelf), vec![clock.clone()], "the wrong tile stayed");
        assert!(
            !items.children().contains(&cpu),
            "a widget nobody asks for is still on the bar"
        );
        assert!(items.children().contains(&clock), "the survivor went too");
    }

    /// Adding a widget to one dock must not reset the stopwatch in another,
    /// and a tile rebuilt is a tile reset.
    pub fn a_widget_that_joined_leaves_the_others_alone() {
        let (items, _, shelf, invoke) = bar();
        shelf.show(&items, &[state("clock")], &invoke);
        let clock = roots(&shelf)[0].clone();

        let did = shelf.show(&items, &[state("clock"), state("water")], &invoke);

        assert_eq!(did, Reconciled { kept: 1, made: 1, gone: 0 });
        assert_eq!(roots(&shelf)[0], clock, "the tile that stayed was remade");
        assert_eq!(roots(&shelf).len(), 2);
    }

    /// Reordering is the case where unparenting would be easiest and worst:
    /// the tiles all survive and only their places change.
    pub fn a_reorder_moves_the_tiles_rather_than_remaking_them() {
        let (items, row, shelf, invoke) = bar();
        shelf.show(&items, &[state("clock"), state("cpu")], &invoke);
        let clock = roots(&shelf)[0].clone();
        let cpu = roots(&shelf)[1].clone();

        let did = shelf.show(&items, &[state("cpu"), state("clock")], &invoke);

        assert_eq!(
            did,
            Reconciled { kept: 2, made: 0, gone: 0 },
            "reordering remade a tile"
        );
        assert_eq!(roots(&shelf), vec![cpu.clone(), clock.clone()]);
        let on_the_bar = items.children();
        let divider = shelf.divider.borrow().clone().expect("a divider").upcast();
        assert_eq!(
            on_the_bar,
            vec![row, divider, cpu, clock],
            "the bar is not in the order the daemon gave"
        );
    }

    pub fn the_divider_only_stands_where_there_is_something_to_divide() {
        let (items, row, shelf, invoke) = bar();
        shelf.show(&items, &[state("clock")], &invoke);
        assert_eq!(items.children().len(), 3, "row, divider, tile");

        shelf.show(&items, &[], &invoke);

        assert_eq!(
            items.children(),
            vec![row],
            "a divider with nothing behind it is a line at the end of the bar"
        );
    }

    /// The divider is kept rather than remade, so that the empty case is not
    /// itself a rebuild.
    pub fn the_divider_that_comes_back_is_the_one_that_left() {
        let (items, _, shelf, invoke) = bar();
        shelf.show(&items, &[state("clock")], &invoke);
        let divider = shelf.divider.borrow().clone().expect("a divider");

        shelf.show(&items, &[], &invoke);
        shelf.show(&items, &[state("clock")], &invoke);

        assert_eq!(shelf.divider.borrow().clone().expect("a divider"), divider);
    }

    pub fn the_shelf_knows_what_is_its_own() {
        let (items, row, shelf, invoke) = bar();
        shelf.show(&items, &[state("clock")], &invoke);

        assert!(!shelf.owns(&row), "the row of icons is not the shelf's");
        for mine in items.children().into_iter().filter(|child| *child != row) {
            assert!(shelf.owns(&mine), "the shelf disowned {mine:?}");
        }
    }

    pub fn a_state_for_a_widget_with_no_tile_is_not_claimed() {
        let (items, _, shelf, invoke) = bar();
        shelf.show(&items, &[state("clock")], &invoke);

        assert!(shelf.update(&state("clock")), "the clock has a tile");
        assert!(
            !shelf.update(&state("water")),
            "a widget with no tile cannot have been updated"
        );
    }
}
