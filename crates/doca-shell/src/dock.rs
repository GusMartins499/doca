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
        let density = picture.scale_factor();
        if let Some(pixbuf) = load_pixbuf(&item.icon, icon_size, density) {
            show_dense(&picture, &pixbuf, density);
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

/// An icon `size` logical pixels across, made at `density` device pixels to
/// each of them — so what comes back is `size * density` wide, and it is
/// `paint_dense` or `show_dense` that puts it on screen at `size`.
pub fn load_pixbuf(name: &str, size: i32, density: i32) -> Option<gdk::gdk_pixbuf::Pixbuf> {
    let from_theme = gtk::IconTheme::default().and_then(|theme| {
        theme
            .load_icon_for_scale(name, size, density, gtk::IconLookupFlags::FORCE_SIZE)
            .ok()
            .flatten()
    });

    from_theme.or_else(|| {
        std::path::Path::new(name)
            .is_absolute()
            .then(|| scaled_from_file(name, size * density))
            .flatten()
    })
}

/// Paint a picture made at `density` with its corner at logical `(x, y)`.
///
/// A pixbuf has no idea how dense it is: painted as it comes on a HiDPI
/// screen it is doubled by the surface and comes out twice the size, or was
/// made small and comes out stretched. So the scale is undone for the length
/// of the paint and the picture lands pixel for pixel, on a device pixel.
pub fn paint_dense(
    cr: &gtk::cairo::Context,
    picture: &gdk::gdk_pixbuf::Pixbuf,
    (x, y): (f64, f64),
    density: i32,
    alpha: f64,
) {
    let density = density as f64;
    let _ = cr.save();
    cr.scale(1.0 / density, 1.0 / density);
    cr.set_source_pixbuf(picture, (x * density).round(), (y * density).round());
    let _ = cr.paint_with_alpha(alpha);
    let _ = cr.restore();
}

/// `paint_dense` for a `gtk::Image`, which takes a surface that knows its
/// own density where a pixbuf would be laid out at one logical pixel each.
pub fn show_dense(image: &gtk::Image, picture: &gdk::gdk_pixbuf::Pixbuf, density: i32) {
    use gdk::prelude::GdkPixbufExt;
    let surface = picture.create_surface(density, None::<&gdk::Window>);
    image.set_from_surface(surface.as_ref());
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

/// What the menu on the empty bar is built from.
pub struct BarLook {
    pub theme: String,
    pub auto_hide: bool,
    pub show_trash: bool,
    /// The widgets on the dock showing now.
    pub widgets: Vec<String>,
}

/// What was chosen in the menu on the empty bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BarChoice {
    Theme(String),
    ToggleAutoHide,
    ShowTrash(bool),
    ToggleWidget(String),
}

/// The menu on the bar itself, for what belongs to no icon (#18, P3).
///
/// The look and the widgets are what a person reaches for first, and they
/// should not need the TOML, or a whole window, to find. Only what one click
/// settles is here; anything that takes a value stays in the preferences,
/// which this ends with.
///
/// It answers with what was chosen rather than doing it, so that all of this
/// can be checked without a daemon.
pub fn bar_menu(look: &BarLook, choose: Rc<dyn Fn(BarChoice)>) -> gtk::Menu {
    let menu = gtk::Menu::new();

    let themes = gtk::Menu::new();
    for name in doca_ipc::THEMES {
        let entry = gtk::CheckMenuItem::with_label(&crate::panel::name_of(name));
        entry.set_draw_as_radio(true);
        entry.set_active(look.theme == name);
        let current = look.theme == name;
        let choosing = choose.clone();
        entry.connect_activate(move |_| {
            if !current {
                choosing(BarChoice::Theme(name.to_string()));
            }
        });
        themes.append(&entry);
    }
    let theme = gtk::MenuItem::with_label("Theme");
    theme.set_submenu(Some(&themes));
    menu.append(&theme);

    let widgets = gtk::Menu::new();
    for id in doca_ipc::WIDGETS {
        let entry = gtk::CheckMenuItem::with_label(&crate::panel::name_of(id));
        entry.set_active(look.widgets.iter().any(|shown| shown == id));
        let choosing = choose.clone();
        entry.connect_activate(move |_| choosing(BarChoice::ToggleWidget(id.to_string())));
        widgets.append(&entry);
    }
    let widget = gtk::MenuItem::with_label("Widgets");
    widget.set_submenu(Some(&widgets));
    menu.append(&widget);

    menu.append(&gtk::SeparatorMenuItem::new());

    let auto_hide = gtk::CheckMenuItem::with_label("Hide automatically");
    auto_hide.set_active(look.auto_hide);
    let choosing = choose.clone();
    auto_hide.connect_activate(move |_| choosing(BarChoice::ToggleAutoHide));
    menu.append(&auto_hide);

    let trash = gtk::CheckMenuItem::with_label("Show the trash");
    trash.set_active(look.show_trash);
    trash.connect_activate(move |entry| choose(BarChoice::ShowTrash(entry.is_active())));
    menu.append(&trash);

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

/// Whether the window in front is fullscreen, and so owns the whole screen.
///
/// Only the active one: a fullscreen video left playing behind the window
/// you are typing in is not in front of anything, and the dock should be.
pub fn covered_by_fullscreen(windows: &[doca_ipc::WindowInfo]) -> bool {
    windows.iter().any(|window| window.active && window.fullscreen)
}

#[cfg(test)]
pub mod tests {
    /// Choosing in the menu on the empty bar says what was chosen, and the
    /// marks in it are what the bar is showing.
    ///
    /// Run by `crate::on_a_display`, which owns the one GTK thread.
    pub fn the_menu_on_the_bar_shows_the_look_and_says_what_was_chosen() {
        use std::cell::RefCell;

        fn entries(menu: &gtk::Menu) -> Vec<gtk::MenuItem> {
            menu.children()
                .into_iter()
                .filter_map(|child| child.downcast::<gtk::MenuItem>().ok())
                .collect()
        }
        fn labelled(menu: &gtk::Menu, label: &str) -> gtk::MenuItem {
            entries(menu)
                .into_iter()
                .find(|entry| entry.label().as_deref() == Some(label))
                .unwrap_or_else(|| panic!("no {label} in the menu"))
        }
        fn opened(entry: gtk::MenuItem) -> gtk::Menu {
            entry
                .submenu()
                .and_then(|menu| menu.downcast::<gtk::Menu>().ok())
                .expect("a submenu")
        }
        fn ticked(entry: &gtk::MenuItem) -> bool {
            entry
                .downcast_ref::<gtk::CheckMenuItem>()
                .expect("a check item")
                .is_active()
        }

        let chosen: Rc<RefCell<Vec<BarChoice>>> = Rc::default();
        let hearing = chosen.clone();
        let menu = bar_menu(
            &BarLook {
                theme: "paper".to_string(),
                auto_hide: true,
                show_trash: false,
                widgets: vec!["water".to_string()],
            },
            Rc::new(move |choice| hearing.borrow_mut().push(choice)),
        );

        let themes = opened(labelled(&menu, "Theme"));
        assert_eq!(entries(&themes).len(), doca_ipc::THEMES.len());
        assert!(ticked(&labelled(&themes, "Paper")));
        assert!(!ticked(&labelled(&themes, "Midnight")));
        let widgets = opened(labelled(&menu, "Widgets"));
        assert_eq!(entries(&widgets).len(), doca_ipc::WIDGETS.len());
        assert!(ticked(&labelled(&widgets, "Water")));
        assert!(!ticked(&labelled(&widgets, "Clock")));
        assert!(ticked(&labelled(&menu, "Hide automatically")));
        assert!(!ticked(&labelled(&menu, "Show the trash")));
        assert!(chosen.borrow().is_empty(), "showing what is true chose something");

        labelled(&themes, "Paper").activate();
        assert!(chosen.borrow().is_empty(), "the theme in force was chosen again");

        labelled(&themes, "Midnight").activate();
        labelled(&widgets, "Clock").activate();
        labelled(&menu, "Hide automatically").activate();
        labelled(&menu, "Show the trash").activate();
        assert_eq!(
            *chosen.borrow(),
            vec![
                BarChoice::Theme("midnight".to_string()),
                BarChoice::ToggleWidget("clock".to_string()),
                BarChoice::ToggleAutoHide,
                BarChoice::ShowTrash(true),
            ]
        );
    }

    /// The whole of what "sharp" means: on a surface twice as dense, a
    /// picture made at twice the pixels arrives without one being invented.
    /// A checkerboard is what resampling would visibly blur to grey.
    #[test]
    fn a_dense_picture_lands_pixel_for_pixel() {
        use gtk::cairo;
        const SIDE: i32 = 8;

        let picture = gdk::gdk_pixbuf::Pixbuf::new(
            gdk::gdk_pixbuf::Colorspace::Rgb,
            true,
            8,
            SIDE,
            SIDE,
        )
        .expect("a pixbuf");
        for y in 0..SIDE as u32 {
            for x in 0..SIDE as u32 {
                let shade = if (x + y) % 2 == 0 { 255 } else { 0 };
                picture.put_pixel(x, y, shade, shade, shade, 255);
            }
        }

        let mut surface =
            cairo::ImageSurface::create(cairo::Format::ARgb32, SIDE, SIDE).expect("no surface");
        surface.set_device_scale(2.0, 2.0);
        {
            let cr = cairo::Context::new(&surface).expect("no context");
            super::paint_dense(&cr, &picture, (0.0, 0.0), 2, 1.0);
        }

        let stride = surface.stride() as usize;
        let pixels = surface.data().expect("the surface is ours");
        for y in 0..SIDE as usize {
            for x in 0..SIDE as usize {
                let expected = if (x + y) % 2 == 0 { 255 } else { 0 };
                assert_eq!(pixels[y * stride + x * 4], expected, "at ({x}, {y})");
            }
        }
    }

    #[test]
    fn only_a_fullscreen_window_in_front_sends_the_dock_away() {
        let window = |active: bool, fullscreen: bool| doca_ipc::WindowInfo {
            id: 1,
            title: String::new(),
            app_id: "mpv".to_string(),
            workspace: 0,
            active,
            fullscreen,
        };
        assert!(covered_by_fullscreen(&[window(false, false), window(true, true)]));
        assert!(!covered_by_fullscreen(&[window(false, true), window(true, false)]), "fullscreen behind, not in front");
        assert!(!covered_by_fullscreen(&[]));
    }

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

    pub fn an_icon_for_a_dense_screen_has_the_pixels_to_fill_it() {
        let docker_sized = write_square_png(1024);

        let dense = load_pixbuf(&docker_sized, ICON_SIZE, 2).unwrap();

        assert_eq!(dense.width(), ICON_SIZE * 2);
        assert_eq!(dense.height(), ICON_SIZE * 2);
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
