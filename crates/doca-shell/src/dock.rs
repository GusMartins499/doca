use std::rc::Rc;

use gtk::prelude::*;
use doca_ipc::{DockItem, DocaProxy, WindowInfo};

pub const ICON_SIZE: i32 = 48;
/// The largest `icon_size` the config can ask for, matching the daemon's clamp.
pub const MAX_ICON_SIZE: i32 = 96;
/// The gap between two icons, and the frame drawn around each.
///
/// Together they are the moulding every icon carries: `ITEM_SPACING +
/// ITEM_PADDING * 2` per slot, which is what a row of thirty icons pays
/// thirty times over. Plank's Dracula theme spends six pixels there
/// (`ItemPadding=1.5`, tenths of the icon size, at 40px icons); twelve put
/// an extra 170px on a full dock and was most of why the bar reached both
/// edges of the screen.
pub const ITEM_SPACING: i32 = 2;
pub const ITEM_PADDING: i32 = 2;
pub const BAR_PADDING: i32 = 10;

pub const MIN_BAR_HEIGHT: i32 = ICON_SIZE + ITEM_PADDING * 2 + BAR_PADDING * 2;
pub const MAX_BAR_HEIGHT: i32 = MAX_ICON_SIZE * 3;
pub const SCREEN_MARGIN: i32 = 8;
pub const MIN_ICON_SIZE: i32 = 24;

pub fn divider() -> gtk::Separator {
    let separator = gtk::Separator::new(gtk::Orientation::Vertical);
    separator.set_widget_name("separator");
    separator.set_size_request(1, ICON_SIZE);
    separator.set_valign(gtk::Align::Center);
    separator
}

pub fn clamp_to_screen(natural: (i32, i32), screen: (i32, i32)) -> (i32, i32) {
    let widest = (screen.0 - SCREEN_MARGIN * 2).max(MIN_BAR_HEIGHT);
    let tallest = MAX_BAR_HEIGHT.min((screen.1 / 3).max(MIN_BAR_HEIGHT));
    (
        natural.0.clamp(MIN_BAR_HEIGHT, widest),
        natural.1.clamp(MIN_BAR_HEIGHT, tallest),
    )
}

/// How wide and tall the bar would like to be, from its contents alone.
///
/// The size it was given last time is cleared first. A widget's preferred size
/// is at least the size it was told to be, so measuring a bar still holding
/// the last answer returns that answer — and since each answer adds the room
/// the lens needs, the bar creeps wider on every rebuild until it covers the
/// screen. Clearing belongs here, where it cannot be forgotten, rather than at
/// the call site, where it was.
pub fn natural_size(bar: &gtk::Box) -> (i32, i32) {
    bar.set_size_request(-1, -1);
    let (_, width) = bar.preferred_width();
    let (_, height) = bar.preferred_height();
    (width, height)
}

/// The way to the apps the bar had no room for.
///
/// The last rung of the ladder in `fit`, and the only one that takes
/// something off the row — so it has to put it somewhere reachable. A dock
/// that simply stops at the edge of the screen loses the apps nobody chose to
/// lose, which is the behaviour this replaced.
///
/// It sits in a slot the width of an icon, because `fit::fits` reserved one
/// for it when it worked out how many icons there was room for: the count it
/// measures is always the icons plus this.
pub fn overflow_control(more: usize, icon_size: i32) -> gtk::EventBox {
    let label = gtk::Label::new(Some("\u{00bb}"));
    label.set_widget_name("overflow");

    let control = gtk::EventBox::new();
    control.set_widget_name("item");
    control.add(&label);
    control.set_size_request(icon_size + ITEM_PADDING * 2, -1);
    control.set_tooltip_text(Some(&format!(
        "{more} more {}",
        if more == 1 { "app" } else { "apps" }
    )));
    control
}

/// Those apps, in the grid the folder stacks already use.
///
/// The same shape and the same arithmetic — `stack::columns_for` and
/// `stack::grid_position` — because it is the same question: a handful of
/// things that have to be picked from, over a bar, without a window. What
/// differs is only what a cell holds and what choosing one does.
///
/// What choosing one does arrives as a closure rather than as the bus proxy,
/// for the reason the folder grid and the widget tiles take one: a grid that
/// cannot be built without a daemon on the other end cannot be checked
/// without one either.
pub fn overflow_grid(
    items: &[DockItem],
    icon_size: i32,
    activate: &crate::stack::Open,
) -> gtk::Menu {
    let menu = gtk::Menu::new();
    menu.set_reserve_toggle_size(false);

    let columns = crate::stack::columns_for(items.len());
    for (index, item) in items.iter().enumerate() {
        let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
        let picture = gtk::Image::new();
        if let Some(pixbuf) = load_pixbuf(&item.icon, icon_size) {
            picture.set_from_pixbuf(Some(&pixbuf));
        }
        column.add(&picture);

        let name = gtk::Label::new(Some(&elide(&item.name, 14)));
        name.set_max_width_chars(14);
        name.set_ellipsize(gtk::pango::EllipsizeMode::End);
        column.add(&name);

        let cell = gtk::MenuItem::new();
        cell.add(&column);
        cell.set_tooltip_text(Some(&item.name));

        let id = item.id.clone();
        let activate = activate.clone();
        cell.connect_activate(move |_| activate(&id));

        let (left, top) = crate::stack::grid_position(index, columns);
        menu.attach(
            &cell,
            left as u32,
            left as u32 + 1,
            top as u32,
            top as u32 + 1,
        );
    }
    menu.show_all();
    menu
}

/// A picture scaled to cover a box of this size, cropping rather than
/// squashing.
///
/// A square album cover in a tile two and a half icons wide has to lose its
/// edges; stretching it to fit is the one thing that makes a cover look
/// wrong at a glance.
pub fn scaled_to_fill(path: &str, width: f64, height: f64) -> Option<gdk::gdk_pixbuf::Pixbuf> {
    let source = gdk::gdk_pixbuf::Pixbuf::from_file(path).ok()?;
    let (from_width, from_height) = (source.width() as f64, source.height() as f64);
    if from_width <= 0.0 || from_height <= 0.0 || width <= 0.0 || height <= 0.0 {
        return None;
    }
    let scale = (width / from_width).max(height / from_height);
    source.scale_simple(
        (from_width * scale).ceil() as i32,
        (from_height * scale).ceil() as i32,
        gdk::gdk_pixbuf::InterpType::Bilinear,
    )
}

pub fn scaled_from_file(path: &str, size: i32) -> Option<gdk::gdk_pixbuf::Pixbuf> {
    gdk::gdk_pixbuf::Pixbuf::from_file_at_scale(path, size, size, true).ok()
}

pub fn load_pixbuf(name: &str, size: i32) -> Option<gdk::gdk_pixbuf::Pixbuf> {
    let from_theme = gtk::IconTheme::default().and_then(|theme| {
        theme
            .load_icon(name, size, gtk::IconLookupFlags::FORCE_SIZE)
            .ok()
            .flatten()
    });

    from_theme.or_else(|| {
        std::path::Path::new(name)
            .is_absolute()
            .then(|| scaled_from_file(name, size))
            .flatten()
    })
}

fn elide(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let kept: String = text.chars().take(limit.saturating_sub(1)).collect();
    format!("{kept}\u{2026}")
}

pub fn context_menu(
    item: &DockItem,
    windows: &[WindowInfo],
    proxy: Rc<DocaProxy<'static>>,
) -> gtk::Menu {
    let menu = gtk::Menu::new();

    if windows.len() > 1 {
        for window in windows {
            let title = if window.title.is_empty() {
                item.name.clone()
            } else {
                window.title.clone()
            };
            let entry = gtk::MenuItem::with_label(&elide(&title, 48));
            entry.set_tooltip_text(Some(&title));
            let id = window.id;
            let proxy = proxy.clone();
            entry.connect_activate(move |_| {
                let proxy = proxy.clone();
                glib::spawn_future_local(async move {
                    let _ = proxy.activate_window(id).await;
                });
            });
            menu.append(&entry);
        }
        menu.append(&gtk::SeparatorMenuItem::new());
    }

    let open = gtk::MenuItem::with_label("New window");
    let id = item.id.clone();
    let p = proxy.clone();
    open.connect_activate(move |_| {
        let id = id.clone();
        let p = p.clone();
        glib::spawn_future_local(async move {
            let _ = p.launch_item(&id).await;
        });
    });
    menu.append(&open);

    let pin_label = if item.pinned { "Unpin" } else { "Pin to dock" };
    let pin = gtk::MenuItem::with_label(pin_label);
    let id = item.id.clone();
    let pinned = item.pinned;
    let p = proxy.clone();
    pin.connect_activate(move |_| {
        let id = id.clone();
        let p = p.clone();
        glib::spawn_future_local(async move {
            let _ = if pinned {
                p.unpin_item(&id).await
            } else {
                p.pin_item(&id).await
            };
        });
    });
    menu.append(&pin);

    if !item.windows.is_empty() {
        menu.append(&gtk::SeparatorMenuItem::new());
        let close = gtk::MenuItem::with_label(if item.windows.len() > 1 {
            "Close all windows"
        } else {
            "Close"
        });
        let windows = item.windows.clone();
        let p = proxy.clone();
        close.connect_activate(move |_| {
            let windows = windows.clone();
            let p = p.clone();
            glib::spawn_future_local(async move {
                for window in windows {
                    let _ = p.close_window(window).await;
                }
            });
        });
        menu.append(&close);
    }

    menu.append(&gtk::SeparatorMenuItem::new());
    menu.append(&preferences());

    menu.show_all();
    menu
}

/// The way into the preferences window from the dock itself.
///
/// This is the hole Plank leaves: its own preferences are only reachable by
/// typing `plank --preferences` in a terminal, so nobody who installed it ever
/// finds them. The dock is the thing the user is looking at when they want to
/// change it, so it is where the way in belongs.
///
/// Spawned rather than linked: the window is a separate process on purpose, so
/// a fault in it cannot take down the bar that reserves the screen edge.
pub fn preferences() -> gtk::MenuItem {
    let entry = gtk::MenuItem::with_label("Preferences…");
    entry.connect_activate(move |_| {
        // Single-instance, so a second click reaches the window already open
        // and brings it forward instead of starting a rival writer.
        match std::process::Command::new(PREFERENCES)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(_) => tracing::info!("opened the preferences window"),
            Err(e) => tracing::error!("cannot open {PREFERENCES}: {e}"),
        }
    });
    entry
}

/// The preferences binary, by name: the workspace installs it beside the
/// others, so whatever found `doca-shell` on PATH finds this too.
const PREFERENCES: &str = "doca-prefs";

/// Files dropped on the row open with whichever app they landed on.
pub fn accept_file_drops(icons: &crate::row::Row) {
    const URI_LIST: u32 = 0;
    let targets = [gtk::TargetEntry::new(
        "text/uri-list",
        gtk::TargetFlags::OTHER_APP,
        URI_LIST,
    )];
    icons
        .area
        .drag_dest_set(gtk::DestDefaults::ALL, &targets, gdk::DragAction::COPY);

    let row = icons.clone();
    icons
        .area
        .connect_drag_data_received(move |_, _, x, _, data, _, _| {
            let Some((_, item)) = row.item_at(x as f64) else {
                return;
            };
            let paths: Vec<String> = data
                .uris()
                .iter()
                .filter_map(|uri| glib::filename_from_uri(uri).ok())
                .map(|(path, _)| path.to_string_lossy().into_owned())
                .collect();
            if paths.is_empty() {
                return;
            }
            let proxy = row.proxy();
            glib::spawn_future_local(async move {
                let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
                if let Err(e) = proxy.open_with(&item.id, &refs).await {
                    tracing::warn!("{} could not open the dropped files: {e}", item.id);
                }
            });
        });
}

#[cfg(test)]
pub mod tests {
    use super::*;

    const SCREEN: (i32, i32) = (1920, 1080);
    #[test]
    fn a_title_short_enough_to_fit_is_left_exactly_as_it_is() {
        assert_eq!(elide("Untitled Document 1", 48), "Untitled Document 1");
    }

    #[test]
    fn a_long_window_title_is_cut_rather_than_stretching_the_menu() {
        let long = "a".repeat(120);

        let short = elide(&long, 48);

        assert_eq!(short.chars().count(), 48);
        assert!(short.ends_with('\u{2026}'));
    }

    #[test]
    fn eliding_counts_characters_not_bytes() {
        let accented = "ação ".repeat(20);

        assert_eq!(elide(&accented, 10).chars().count(), 10);
    }

    fn write_square_png(side: i32) -> String {
        let path = std::env::temp_dir().join(format!("doca-icon-{side}.png"));
        let pixbuf = gdk::gdk_pixbuf::Pixbuf::new(
            gdk::gdk_pixbuf::Colorspace::Rgb,
            true,
            8,
            side,
            side,
        )
        .unwrap();
        pixbuf.fill(0xff0000ff);
        pixbuf.savev(&path, "png", &[]).unwrap();
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn an_icon_declared_by_absolute_path_is_scaled_down_to_the_dock_size() {
        let docker_sized = write_square_png(1024);

        let pixbuf = scaled_from_file(&docker_sized, ICON_SIZE).unwrap();

        assert_eq!(pixbuf.width(), ICON_SIZE);
        assert_eq!(pixbuf.height(), ICON_SIZE);
    }

    #[test]
    fn a_small_icon_is_scaled_up_rather_than_left_ragged() {
        let tiny = write_square_png(16);

        let pixbuf = scaled_from_file(&tiny, ICON_SIZE).unwrap();

        assert_eq!(pixbuf.width(), ICON_SIZE);
    }

    #[test]
    fn a_missing_icon_file_yields_nothing_instead_of_panicking() {
        assert!(scaled_from_file("/nonexistent/icon.png", ICON_SIZE).is_none());
    }

    #[test]
    fn the_moulding_each_icon_carries_is_no_heavier_than_planks() {
        // Plank's Dracula theme spends `ItemPadding=1.5` — tenths of the
        // icon size, so six pixels at its 40px icons. Thirty icons pay for
        // this thirty times, and it was most of the width that made the bar
        // reach both edges of the screen.
        assert!(ITEM_SPACING + ITEM_PADDING * 2 <= 6);
    }

    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    fn app(id: &str) -> DockItem {
        DockItem {
            id: id.into(),
            name: format!("The {id}"),
            icon: "application-x-executable".into(),
            pinned: true,
            windows: Vec::new(),
            active: false,
        }
    }

    /// Every app that came off the row is in the grid, and choosing one asks
    /// for that one.
    ///
    /// The whole point of the last rung: what does not fit is put somewhere
    /// reachable rather than cut off the end. A grid that quietly held six of
    /// the eight would be the same loss with more steps.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn every_app_that_did_not_fit_is_in_the_grid() {
        let over: Vec<DockItem> = (0..8).map(|at| app(&format!("app{at}"))).collect();
        let chosen = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let noting = chosen.clone();
        let activate: crate::stack::Open =
            Rc::new(move |id: &str| noting.borrow_mut().push(id.to_string()));

        let grid = overflow_grid(&over, ICON_SIZE, &activate);

        let cells = grid.children();
        assert_eq!(cells.len(), over.len(), "the grid lost an app on the way");

        for cell in &cells {
            if let Some(item) = cell.downcast_ref::<gtk::MenuItem>() {
                item.emit_activate();
            }
        }
        assert_eq!(
            *chosen.borrow(),
            over.iter().map(|item| item.id.clone()).collect::<Vec<_>>(),
            "choosing a cell asked for the wrong app"
        );
    }

    /// And the control says how many there are, because a bare chevron is a
    /// thing you have to click to find out about.
    ///
    /// Run by `crate::on_a_display`: it builds a widget, and a widget needs a
    /// GTK that has been started.
    pub fn the_control_says_how_many_it_is_hiding() {
        assert!(overflow_control(1, 48).tooltip_text().unwrap().contains("1 more app"));
        assert!(overflow_control(8, 48).tooltip_text().unwrap().contains("8 more apps"));
    }

    pub fn measuring_a_bar_twice_gives_the_same_answer_both_times() {
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, ITEM_SPACING);
        for _ in 0..8 {
            let icon = gtk::Image::from_icon_name(Some("folder"), gtk::IconSize::Dnd);
            icon.set_pixel_size(ICON_SIZE);
            bar.add(&icon);
        }

        let first = natural_size(&bar);
        // what a rebuild does with the answer
        bar.set_size_request(
            first.0 + crate::magnify::edge_room(ICON_SIZE as f64, 1.6),
            first.1,
        );
        let again = natural_size(&bar);

        assert_eq!(
            first, again,
            "the bar answered with the width it was given, and would creep wider on every rebuild"
        );
    }

    #[test]
    fn a_bar_that_fits_the_screen_is_left_alone() {
        let fits = (900, MIN_BAR_HEIGHT);

        assert_eq!(clamp_to_screen(fits, SCREEN), fits);
    }

    #[test]
    fn a_thousand_pixel_icon_can_never_make_the_bar_swallow_the_screen() {
        let docker_desktop_icon_is_1024_square = (4000, 1080);

        let (width, height) = clamp_to_screen(docker_desktop_icon_is_1024_square, SCREEN);

        assert!(width <= SCREEN.0);
        assert!(height <= MAX_BAR_HEIGHT);
        assert!(height < SCREEN.1 / 3);
    }

    #[test]
    fn twenty_eight_pinned_apps_cannot_widen_the_bar_past_the_screen() {
        let slot = ICON_SIZE + ITEM_PADDING * 2 + ITEM_SPACING;
        let twenty_eight = (28 * slot, MIN_BAR_HEIGHT);

        let (width, _) = clamp_to_screen(twenty_eight, SCREEN);

        assert!(width <= SCREEN.0 - SCREEN_MARGIN);
    }

    #[test]
    fn an_empty_bar_is_never_clamped_to_nothing() {
        let (width, height) = clamp_to_screen((0, 0), SCREEN);

        assert_eq!(width, MIN_BAR_HEIGHT);
        assert_eq!(height, MIN_BAR_HEIGHT);
    }

    #[test]
    fn a_very_short_screen_still_leaves_room_for_one_row_of_icons() {
        let (_, height) = clamp_to_screen((900, 4000), (800, 200));

        assert_eq!(height, MIN_BAR_HEIGHT);
    }
}
