use std::rc::Rc;

use gtk::prelude::*;
use doca_ipc::{DockItem, DocaProxy, WindowInfo};

pub const ICON_SIZE: i32 = 48;
/// The largest `icon_size` the config can ask for, matching the daemon's clamp.
pub const MAX_ICON_SIZE: i32 = 96;
pub const ITEM_SPACING: i32 = 4;
pub const ITEM_PADDING: i32 = 4;
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

/// The icon size that fits the row *and* the room the lens needs to open in.
///
/// Sizing the icons to fill the screen exactly leaves the lens nowhere to
/// grow: the row asks for more width than the bar has, and GTK answers by
/// squeezing every icon that is not under the pointer. So the space the lens
/// will want is part of what the row has to fit, which costs a few pixels of
/// icon on a crowded dock and buys magnification that does not shove.
pub fn icon_size_for(
    item_count: i32,
    widget_count: i32,
    screen_width: i32,
    preferred: i32,
    magnification: f64,
) -> i32 {
    let preferred = preferred.clamp(MIN_ICON_SIZE, MAX_ICON_SIZE);
    if item_count <= 0 {
        return preferred;
    }
    let reserved = ITEM_SPACING * 2
        + widget_count * (crate::widget_tile::TILE_WIDTH + ITEM_SPACING)
        + BAR_PADDING * 2
        + SCREEN_MARGIN * 2;
    let available = (screen_width - reserved).max(0);
    let per_item_frame = ITEM_SPACING + ITEM_PADDING * 2;

    // count * (icon + frame) + room at both ends <= available
    let growth = crate::magnify::edge_reach(magnification) * 2.0;
    let room = (available - item_count * per_item_frame).max(0) as f64;
    let per_item = (room / (item_count as f64 + growth)).floor() as i32;

    per_item.clamp(MIN_ICON_SIZE, preferred)
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

    menu.show_all();
    menu
}

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
    const NO_LENS: f64 = 1.0;
    const LENS: f64 = 1.6;

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
    fn a_smaller_preferred_size_is_honoured_even_when_there_is_room_to_spare() {
        assert_eq!(icon_size_for(4, 0, SCREEN.0, 32, NO_LENS), 32);
    }

    #[test]
    fn an_icon_size_larger_than_the_default_is_honoured_when_there_is_room() {
        assert_eq!(icon_size_for(6, 2, SCREEN.0, 72, NO_LENS), 72);
    }

    #[test]
    fn no_config_can_ask_for_an_icon_larger_than_the_dock_allows() {
        assert_eq!(icon_size_for(4, 0, SCREEN.0, 400, NO_LENS), MAX_ICON_SIZE);
    }

    #[test]
    fn a_crowded_dock_of_twenty_nine_apps_still_leaves_the_icons_legible() {
        let plank_sized_row = icon_size_for(29, 4, SCREEN.0, ICON_SIZE, LENS);

        assert!(
            plank_sized_row >= 32,
            "{plank_sized_row}px icons are too small to recognise"
        );
    }

    #[test]
    fn a_row_sized_with_the_lens_on_leaves_the_lens_somewhere_to_open() {
        for count in 1..60 {
            let size = icon_size_for(count, 4, SCREEN.0, ICON_SIZE, LENS);
            let reserved = ITEM_SPACING * 2
                + 4 * (crate::widget_tile::TILE_WIDTH + ITEM_SPACING)
                + BAR_PADDING * 2
                + SCREEN_MARGIN * 2;
            let open = count * (size + ITEM_PADDING * 2 + ITEM_SPACING)
                + crate::magnify::edge_room(size as f64, LENS) * 2
                + reserved;

            if size > MIN_ICON_SIZE {
                assert!(
                    open <= SCREEN.0,
                    "{count} icons at {size}px need {open}px of {}px with the lens open",
                    SCREEN.0
                );
            }
        }
    }

    #[test]
    fn turning_the_lens_off_gives_the_icons_the_room_it_was_holding() {
        let with_lens = icon_size_for(29, 4, SCREEN.0, ICON_SIZE, LENS);
        let without = icon_size_for(29, 4, SCREEN.0, ICON_SIZE, NO_LENS);

        assert!(
            without > with_lens,
            "a dock with no magnification should spend that room on the icons"
        );
    }

    #[test]
    fn a_preferred_size_never_overrides_the_need_to_fit() {
        let crowded = icon_size_for(40, 4, SCREEN.0, ICON_SIZE, NO_LENS);

        assert!(crowded < ICON_SIZE);
    }

    #[test]
    fn a_handful_of_apps_keeps_icons_at_full_size() {
        assert_eq!(icon_size_for(6, 2, SCREEN.0, ICON_SIZE, NO_LENS), ICON_SIZE);
    }

    #[test]
    fn twenty_eight_pinned_apps_shrink_the_icons_instead_of_overflowing() {
        let size = icon_size_for(28, 4, SCREEN.0, ICON_SIZE, NO_LENS);

        assert!(size < ICON_SIZE);
        assert!(size >= MIN_ICON_SIZE);
    }

    #[test]
    fn the_shrunken_icons_actually_fit_the_screen_they_were_sized_for() {
        for count in 1..60 {
            let size = icon_size_for(count, 4, SCREEN.0, ICON_SIZE, NO_LENS);
            let reserved = ITEM_SPACING * 2
                + 4 * (crate::widget_tile::TILE_WIDTH + ITEM_SPACING)
                + BAR_PADDING * 2
                + SCREEN_MARGIN * 2;
            let used = count * (size + ITEM_PADDING * 2 + ITEM_SPACING) + reserved;

            if size > MIN_ICON_SIZE {
                assert!(
                    used <= SCREEN.0,
                    "{count} apps at {size}px need {used}px of {}px",
                    SCREEN.0
                );
            }
        }
    }

    #[test]
    fn icons_never_shrink_below_the_point_of_being_recognisable() {
        assert_eq!(icon_size_for(500, 4, SCREEN.0, ICON_SIZE, NO_LENS), MIN_ICON_SIZE);
    }

    #[test]
    fn an_empty_dock_does_not_divide_by_zero_sizing_its_icons() {
        assert_eq!(icon_size_for(0, 0, SCREEN.0, ICON_SIZE, NO_LENS), ICON_SIZE);
    }

    /// Run by `crate::on_a_display`, which owns the one GTK thread.
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
