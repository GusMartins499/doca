use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk::prelude::*;
use doca_ipc::FolderEntry;

pub const PREFIX: &str = "folder:";
pub const COLUMNS: i32 = 5;

pub fn is_folder(id: &str) -> bool {
    id.starts_with(PREFIX)
}
pub const ENTRY_ICON: i32 = 32;

/// How wide every cell's name is, in characters.
///
/// A width rather than a cap: the grid is then as wide as its shape and not as
/// wide as its longest filename, which is what lets a grid be opened at the
/// size its answer will take before the answer is known.
pub const LABEL_CHARS: i32 = 14;

/// How long a grid is left alone after it goes up, before it may be taken
/// down to be resized.
///
/// Measured, and the reason this is not simply "straight away": a grid put up
/// by a press and taken down and up again before the *release* of that same
/// click is dismissed by that release — the release lands on the new menu and
/// closes it. With a folder that reads instantly the whole dance fits inside
/// one click, and the grid opened one time in three. Longer than a click is
/// held, so the release is always spent before anything moves.
pub const SETTLED: Duration = Duration::from_millis(250);

/// How faint a cell is while it is still only the promise of one.
const WAITING: f64 = 0.3;

/// How many cells a folder nobody has opened yet is drawn with: one row.
pub const UNKNOWN: usize = COLUMNS as usize;

/// What a cell does when it is chosen: open that path.
///
/// A closure rather than the bus proxy, for the same reason the widget tiles
/// take one — the grid has no business knowing what a path is opened *with*,
/// and a grid that cannot be built without a daemon on the other end cannot be
/// checked without one either.
pub type Open = Rc<dyn Fn(&str)>;

/// How the grid is put on screen: the click that opened it, applied again.
///
/// The same closure opens it and reopens it, so the two cannot drift apart —
/// a grid that came back somewhere other than where it was opened would be
/// worse than one that never came back at all. It also spares this file any
/// knowledge of GDK events, which a test has none of to give.
pub type Show = Rc<dyn Fn(&gtk::Menu)>;

/// The shape a grid of `count` entries takes: columns by rows.
///
/// Every cell is the same size, so this is the whole of how big a grid is —
/// which is what makes it answerable before the folder has been read.
pub fn grid_of(count: usize) -> (i32, i32) {
    let columns = columns_for(count);
    let rows = (count.max(1) as i32 + columns - 1) / columns;
    (columns, rows)
}

/// What each folder held the last time it was opened.
///
/// Not a nicety. A GTK menu takes its window's size when it pops up and never
/// again: cells put into one that is already open are not shown but *scrolled*
/// to, and the only way to resize it is to pop it down and up again, which
/// blinks. So a skeleton of the right shape is what keeps the common case —
/// a pinned folder, opened over and over — from blinking at all.
///
/// It lives as long as the bar does, not as long as a menu: a menu is built
/// and dropped on every click.
#[derive(Clone, Default)]
pub struct Remembered(Rc<RefCell<HashMap<String, usize>>>);

impl Remembered {
    pub fn of(&self, id: &str) -> usize {
        self.0.borrow().get(id).copied().unwrap_or(UNKNOWN)
    }

    pub fn note(&self, id: &str, count: usize) {
        self.0.borrow_mut().insert(id.to_string(), count);
    }
}

pub fn grid_position(index: usize, columns: i32) -> (i32, i32) {
    let columns = columns.max(1);
    let index = index as i32;
    (index % columns, index / columns)
}

pub fn columns_for(count: usize) -> i32 {
    if count <= 4 {
        count.max(1) as i32
    } else {
        COLUMNS
    }
}

fn entry_icon(entry: &FolderEntry) -> gtk::Image {
    let name = if entry.is_dir {
        "folder".to_string()
    } else {
        let (content_type, _) = gtk::gio::content_type_guess(Some(&entry.name), &[]);
        gtk::gio::content_type_get_generic_icon_name(&content_type)
            .map(|name| name.to_string())
            .unwrap_or_else(|| "text-x-generic".to_string())
    };

    let image = gtk::IconTheme::default()
        .and_then(|theme| {
            theme
                .load_icon(&name, ENTRY_ICON, gtk::IconLookupFlags::FORCE_SIZE)
                .ok()
                .flatten()
        })
        .map(|pixbuf| gtk::Image::from_pixbuf(Some(&pixbuf)))
        .unwrap_or_else(|| {
            gtk::Image::from_icon_name(Some("text-x-generic"), gtk::IconSize::Dnd)
        });
    image.set_pixel_size(ENTRY_ICON);
    image
}

/// The name under a cell, at the one width every cell has.
fn cell_label(name: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(name));
    label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    label.set_width_chars(LABEL_CHARS);
    label.set_max_width_chars(LABEL_CHARS);
    label.set_widget_name("stack-label");
    label
}

/// The cell of a folder that has not answered yet: the shape of a cell at a
/// fraction of its weight.
///
/// Dimmed rather than styled. The grid is a GTK menu and wears whatever the
/// desktop's menus wear, so a colour chosen here would be a guess about a
/// background this code never picked; an opacity is faint against any of them.
fn waiting_cell() -> gtk::MenuItem {
    let block = gtk::Image::from_icon_name(Some("text-x-generic"), gtk::IconSize::Dnd);
    block.set_pixel_size(ENTRY_ICON);

    // A space, not nothing: an empty label is no lines tall, and the cells
    // would grow by a line when the names arrived.
    let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
    column.add(&block);
    column.add(&cell_label(" "));

    let cell = gtk::MenuItem::new();
    cell.add(&column);
    cell.set_opacity(WAITING);
    // There is nothing to open yet, and a cell that looks pressable but is
    // not is worse than one that plainly is not.
    cell.set_sensitive(false);
    cell
}

/// The grid, before anyone has said what is in the folder.
///
/// Opening waits on nothing: this is on screen in the frame the click landed
/// in, and `fill` puts the entries into it when they arrive. Reading a folder
/// is the one thing a click can start whose cost has no ceiling — a disk that
/// is asleep, a mount that is not local — and the shell can put none on it.
pub fn opening(expected: usize) -> gtk::Menu {
    let menu = gtk::Menu::new();
    menu.set_widget_name("stack");

    let (columns, _) = grid_of(expected);
    for index in 0..expected.max(1) {
        let (left, top) = grid_position(index, columns);
        menu.attach(
            &waiting_cell(),
            left as u32,
            left as u32 + 1,
            top as u32,
            top as u32 + 1,
        );
    }

    menu.show_all();
    menu
}

/// Put the folder into the grid that is already on screen.
///
/// `expected` is what the grid was opened for. When the answer has the same
/// shape — a pinned folder opened again, which is nearly every open — the
/// cells are simply swapped and nothing moves. When it does not, the grid has
/// to be popped down and up again: a menu keeps the window it sized at popup,
/// so cells beyond that window are not shown but scrolled to, and a grid that
/// quietly hid forty of its sixty files would be worse than one that blinked.
pub fn fill(
    menu: &gtk::Menu,
    expected: usize,
    entries: &[FolderEntry],
    open: &Open,
    show: &Show,
    opened: Instant,
) {
    for cell in menu.children() {
        menu.remove(&cell);
    }

    if entries.is_empty() {
        let empty = gtk::MenuItem::with_label("empty");
        empty.set_sensitive(false);
        menu.append(&empty);
        menu.show_all();
        // Nothing about an empty folder is the shape of a grid of cells.
        reopen(menu, show, opened);
        return;
    }

    let columns = columns_for(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
        column.add(&entry_icon(entry));
        column.add(&cell_label(&entry.name));

        let cell = gtk::MenuItem::new();
        cell.add(&column);
        cell.set_tooltip_text(Some(&entry.name));

        let path = entry.path.clone();
        let open = open.clone();
        cell.connect_activate(move |_| open(&path));

        let (left, top) = grid_position(index, columns);
        menu.attach(&cell, left as u32, left as u32 + 1, top as u32, top as u32 + 1);
    }

    menu.show_all();
    if grid_of(expected) != grid_of(entries.len()) {
        reopen(menu, show, opened);
    }
}

/// Take the grid down and put it up again, which is the only way a menu
/// changes the size of its window.
///
/// Not if it is already gone: a folder that took long enough to read that the
/// pointer went elsewhere must not have its grid thrown back up afterwards.
fn reopen(menu: &gtk::Menu, show: &Show, opened: Instant) {
    if !menu.is_visible() {
        return;
    }
    let menu = menu.clone();
    let show = show.clone();
    let swap = move || {
        if !menu.is_visible() {
            return;
        }
        menu.popdown();
        // Up again on the next turn, not this one: the one just dismissed is
        // still handing back its grab, and a popup into that is swallowed.
        let menu = menu.clone();
        let show = show.clone();
        glib::idle_add_local_once(move || show(&menu));
    };

    let waited = opened.elapsed();
    if waited >= SETTLED {
        swap();
    } else {
        glib::timeout_add_local_once(SETTLED - waited, swap);
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use std::cell::Cell;

    fn entries(count: usize) -> Vec<FolderEntry> {
        (0..count)
            .map(|index| FolderEntry {
                name: format!("a-file-{index}.txt"),
                path: format!("/tmp/a-file-{index}.txt"),
                is_dir: index == 0,
            })
            .collect()
    }

    fn opens_nothing() -> Open {
        Rc::new(|_: &str| {})
    }

    /// Let GTK get on with whatever the last call asked for.
    fn settle() {
        for _ in 0..200 {
            if !gtk::events_pending() {
                break;
            }
            gtk::main_iteration_do(false);
        }
    }

    /// Somewhere for a grid to hang off, and the click that opened it.
    ///
    /// `popup_at_pointer` reads the pointer out of an event, and a test has
    /// none to give it — so these pop up against a widget, and the event the
    /// real code would pass is `None` here.
    fn popped_up(grid: &gtk::Menu) -> gtk::Window {
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.show_all();
        settle();
        shown_against(&window)(grid);
        settle();
        window
    }

    /// The way these tests put a grid on screen — against a widget, since a
    /// test has no click to read a pointer out of. `fill` reopens through
    /// this same closure, which is the path the real click takes too.
    fn shown_against(window: &gtk::Window) -> Show {
        counted_against(window).0
    }

    /// The same, keeping count: one call is the click, a second is the grid
    /// having to be thrown down and put up again. Counting is the only way to
    /// tell a grid that never blinked from one that blinked back to the size
    /// it already was.
    fn counted_against(window: &gtk::Window) -> (Show, Rc<Cell<usize>>) {
        let window = window.clone();
        let times = Rc::new(Cell::new(0));
        let counting = times.clone();
        let show: Show = Rc::new(move |menu: &gtk::Menu| {
            counting.set(counting.get() + 1);
            menu.popup_at_widget(
                &window,
                gdk::Gravity::NorthWest,
                gdk::Gravity::NorthWest,
                None,
            );
        });
        (show, times)
    }

    /// How big the grid's window is.
    fn window_of(grid: &gtk::Menu) -> (i32, i32) {
        let allocation = grid
            .toplevel()
            .map(|top| top.allocation())
            .expect("a grid that is up has a window");
        (allocation.width(), allocation.height())
    }

    /// The whole of the item: the grid is up before the folder was read.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_grid_opens_before_the_folder_has_been_read() {
        let grid = opening(5);
        let window = popped_up(&grid);

        assert!(
            grid.is_visible(),
            "the click put nothing on screen, and the folder has not answered yet"
        );
        assert_eq!(grid.children().len(), 5, "the skeleton is not the shape it was asked for");
        assert!(
            grid.children().iter().all(|cell| !cell.is_sensitive()),
            "a cell that holds nothing yet can still be chosen"
        );

        // The listing lands in the grid that is already up, rather than in a
        // second one.
        fill(&grid, 5, &entries(5), &opens_nothing(), &shown_against(&window), Instant::now() - SETTLED);
        settle();

        assert!(grid.is_visible(), "filling the grid closed it");
        assert_eq!(grid.children().len(), 5);
        assert!(
            grid.children().iter().all(|cell| cell.is_sensitive()),
            "the entries arrived but cannot be opened"
        );
        grid.popdown();
        window.close();
        settle();
    }

    /// Why `Remembered` is load-bearing and not a nicety.
    ///
    /// A folder opened at the shape it turns out to have does not move at all:
    /// same window, same size, the cells swapped underneath. This is the open
    /// that happens over and over, and it is the one that must not blink.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_folder_opened_at_the_shape_it_turns_out_to_have_does_not_move() {
        let grid = opening(23);
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.show_all();
        settle();
        let (show, shown) = counted_against(&window);
        show(&grid);
        settle();
        let before = window_of(&grid);

        fill(&grid, 23, &entries(23), &opens_nothing(), &show, Instant::now() - SETTLED);
        settle();

        assert_eq!(
            shown.get(),
            1,
            "the grid was taken down and put up again for a shape it already had"
        );
        assert_eq!(
            window_of(&grid),
            before,
            "the grid resized under the pointer even though the guess was right"
        );
        grid.popdown();
        window.close();
        settle();
    }

    /// And the other half: a guess that was wrong still shows the whole
    /// folder, rather than hiding most of it behind a scroll arrow.
    ///
    /// A menu keeps the window it sized at popup. Cells added past it are not
    /// shown but scrolled to — twenty-three files arriving in a grid opened
    /// for five left one row on screen with an arrow under it, every entry
    /// present, counted, and invisible.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_folder_larger_than_the_grid_it_opened_is_still_shown_whole() {
        let grid = opening(UNKNOWN);
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.show_all();
        settle();
        let (show, shown) = counted_against(&window);
        show(&grid);
        settle();

        fill(&grid, UNKNOWN, &entries(23), &opens_nothing(), &show, Instant::now() - SETTLED);
        settle();

        assert_eq!(shown.get(), 2, "the grid never came back at its new size");

        let (_, natural) = grid.preferred_size();
        let (width, height) = window_of(&grid);
        assert!(
            width >= natural.width && height >= natural.height,
            "the grid is {width}x{height} for {}x{} of folder: the rest is \
             behind a scroll arrow",
            natural.width,
            natural.height
        );
        grid.popdown();
        window.close();
        settle();
    }

    /// An empty folder has to say so, not stay a skeleton for ever.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_folder_with_nothing_in_it_says_so_rather_than_waiting_for_ever() {
        let grid = opening(UNKNOWN);
        let window = popped_up(&grid);

        fill(&grid, UNKNOWN, &[], &opens_nothing(), &shown_against(&window), Instant::now() - SETTLED);
        settle();

        assert_eq!(grid.children().len(), 1);
        assert!(!grid.children()[0].is_sensitive());
        grid.popdown();
        window.close();
        settle();
    }

    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_folder_opened_before_is_drawn_at_the_size_it_was() {
        let folders = Remembered::default();
        let id = "folder:/home/someone/Downloads";

        assert_eq!(folders.of(id), UNKNOWN, "a folder nobody opened has no size to use");
        assert_eq!(opening(folders.of(id)).children().len(), UNKNOWN);

        folders.note(id, 23);

        assert_eq!(folders.of(id), 23);
        assert_eq!(
            opening(folders.of(id)).children().len(),
            23,
            "the second open guessed again instead of using what it saw"
        );
    }

    /// A folder that emptied since it was last opened still has to open.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn a_grid_always_has_a_cell_to_show_however_little_is_expected() {
        assert_eq!(opening(0).children().len(), 1);
    }

    #[test]
    fn a_grid_is_as_many_rows_as_its_entries_need() {
        assert_eq!(grid_of(0), (1, 1));
        assert_eq!(grid_of(1), (1, 1));
        assert_eq!(grid_of(4), (4, 1));
        assert_eq!(grid_of(5), (COLUMNS, 1));
        assert_eq!(grid_of(6), (COLUMNS, 2));
        assert_eq!(grid_of(23), (COLUMNS, 5));
    }

    /// The question `fill` actually asks: is what arrived the shape of what
    /// was opened for it?
    #[test]
    fn folders_of_different_sizes_can_still_share_a_shape() {
        assert_eq!(grid_of(21), grid_of(25), "both are five rows of five");
        assert_ne!(grid_of(5), grid_of(6));
    }

    #[test]
    fn a_folder_item_is_told_apart_from_an_app_and_a_window() {
        assert!(is_folder("folder:/home/someone/Downloads"));
        assert!(!is_folder("code"));
        assert!(!is_folder("window:42"));
        assert!(!is_folder("trash"));
    }

    #[test]
    fn entries_fill_left_to_right_then_wrap() {
        assert_eq!(grid_position(0, 5), (0, 0));
        assert_eq!(grid_position(4, 5), (4, 0));
        assert_eq!(grid_position(5, 5), (0, 1));
        assert_eq!(grid_position(7, 5), (2, 1));
    }

    #[test]
    fn no_two_entries_land_on_the_same_cell() {
        let mut seen = std::collections::HashSet::new();

        for index in 0..60 {
            assert!(seen.insert(grid_position(index, COLUMNS)));
        }
    }

    #[test]
    fn a_zero_column_grid_does_not_divide_by_zero() {
        assert_eq!(grid_position(3, 0), (0, 3));
    }

    #[test]
    fn a_short_folder_stays_on_one_row_rather_than_padding_to_five() {
        assert_eq!(columns_for(1), 1);
        assert_eq!(columns_for(3), 3);
        assert_eq!(columns_for(4), 4);
    }

    #[test]
    fn a_long_folder_wraps_at_the_grid_width() {
        assert_eq!(columns_for(5), COLUMNS);
        assert_eq!(columns_for(60), COLUMNS);
    }

    #[test]
    fn an_empty_folder_still_has_a_column_count() {
        assert_eq!(columns_for(0), 1);
    }
}
