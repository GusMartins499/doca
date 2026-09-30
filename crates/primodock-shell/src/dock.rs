use std::rc::Rc;

use gtk::prelude::*;
use primodock_ipc::{DockItem, PrimoDockProxy, WindowInfo};

pub const ICON_SIZE: i32 = 48;
pub const ITEM_SPACING: i32 = 6;
pub const ITEM_PADDING: i32 = 8;
pub const BAR_PADDING: i32 = 10;

pub const SWITCHER_WIDTH: i32 = 96;
pub const MIN_BAR_HEIGHT: i32 = ICON_SIZE + ITEM_PADDING * 2 + BAR_PADDING * 2;
pub const MAX_BAR_HEIGHT: i32 = MIN_BAR_HEIGHT * 2;
pub const SCREEN_MARGIN: i32 = 8;
pub const MIN_ICON_SIZE: i32 = 24;

pub fn switcher_is_useful(environment_count: usize) -> bool {
    environment_count > 1
}

pub fn divider() -> gtk::Separator {
    let separator = gtk::Separator::new(gtk::Orientation::Vertical);
    separator.set_widget_name("separator");
    separator.set_size_request(1, ICON_SIZE);
    separator.set_valign(gtk::Align::Center);
    separator
}

pub fn icon_size_for(
    item_count: i32,
    widget_count: i32,
    screen_width: i32,
    preferred: i32,
) -> i32 {
    let preferred = preferred.clamp(MIN_ICON_SIZE, ICON_SIZE);
    if item_count <= 0 {
        return preferred;
    }
    let reserved = SWITCHER_WIDTH
        + ITEM_SPACING * 2
        + widget_count * (crate::widget_tile::TILE_WIDTH + ITEM_SPACING)
        + BAR_PADDING * 2
        + SCREEN_MARGIN * 2;
    let available = (screen_width - reserved).max(0);
    let per_item = available / item_count - ITEM_SPACING - ITEM_PADDING * 2;
    per_item.clamp(MIN_ICON_SIZE, preferred)
}

pub fn clamp_to_screen(natural: (i32, i32), screen: (i32, i32)) -> (i32, i32) {
    let widest = (screen.0 - SCREEN_MARGIN * 2).max(SWITCHER_WIDTH);
    let tallest = MAX_BAR_HEIGHT.min((screen.1 / 3).max(MIN_BAR_HEIGHT));
    (
        natural.0.clamp(SWITCHER_WIDTH, widest),
        natural.1.clamp(MIN_BAR_HEIGHT, tallest),
    )
}

pub fn natural_size(bar: &gtk::Box) -> (i32, i32) {
    let (_, width) = bar.preferred_width();
    let (_, height) = bar.preferred_height();
    (width, height)
}

pub fn environment_switcher(
    name: &str,
    proxy: Rc<PrimoDockProxy<'static>>,
) -> gtk::Widget {
    let label = gtk::Label::new(Some(name));
    label.set_widget_name("environment-name");
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label.set_max_width_chars(10);

    let chip = gtk::EventBox::new();
    chip.set_widget_name("environment");
    chip.set_tooltip_text(Some("Click to switch environment"));
    chip.set_size_request(SWITCHER_WIDTH, ICON_SIZE);
    chip.add(&label);

    chip.connect_button_press_event(move |_, _| {
        let proxy = proxy.clone();
        glib::spawn_future_local(async move {
            if let Err(e) = proxy.cycle_environment().await {
                tracing::warn!("cannot cycle environment: {e}");
            }
        });
        glib::Propagation::Stop
    });

    chip.upcast()
}

pub fn scaled_from_file(path: &str, size: i32) -> Option<gdk::gdk_pixbuf::Pixbuf> {
    gdk::gdk_pixbuf::Pixbuf::from_file_at_scale(path, size, size, true).ok()
}

fn icon_widget(name: &str, size: i32) -> gtk::Image {
    let from_theme = gtk::IconTheme::default().and_then(|theme| {
        theme
            .load_icon(name, size, gtk::IconLookupFlags::FORCE_SIZE)
            .ok()
            .flatten()
    });

    let pixbuf = from_theme.or_else(|| {
        std::path::Path::new(name)
            .is_absolute()
            .then(|| scaled_from_file(name, size))
            .flatten()
    });

    let image = match pixbuf {
        Some(pixbuf) => gtk::Image::from_pixbuf(Some(&pixbuf)),
        None => {
            gtk::Image::from_icon_name(Some("application-x-executable"), gtk::IconSize::Dnd)
        }
    };
    image.set_pixel_size(size);
    image
}

fn elide(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let kept: String = text.chars().take(limit.saturating_sub(1)).collect();
    format!("{kept}\u{2026}")
}

fn context_menu(
    item: &DockItem,
    windows: &[WindowInfo],
    proxy: Rc<PrimoDockProxy<'static>>,
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

pub fn indicator_state(item: &DockItem) -> &'static str {
    match (item.windows.is_empty(), item.active) {
        (true, _) => "indicator-idle",
        (false, false) => "indicator",
        (false, true) => "indicator-active",
    }
}

pub struct ItemWidgets {
    pub root: gtk::Widget,
    pub image: gtk::Image,
}

pub fn item_button(
    item: &DockItem,
    size: i32,
    proxy: Rc<PrimoDockProxy<'static>>,
) -> ItemWidgets {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let image = icon_widget(&item.icon, size);
    column.add(&image);

    let indicator = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    indicator.set_widget_name(indicator_state(item));
    indicator.set_size_request(6, 3);
    indicator.set_halign(gtk::Align::Center);
    indicator.set_margin_top(3);
    column.add(&indicator);

    let button = gtk::EventBox::new();
    button.add_events(gdk::EventMask::POINTER_MOTION_MASK);
    button.set_widget_name("item");
    button.set_tooltip_text(Some(&item.name));
    button.add(&column);

    let id = item.id.clone();
    let menu_item = item.clone();
    let menu_proxy = proxy.clone();
    let proxy_for_drop = proxy.clone();
    button.connect_button_press_event(move |_, event| {
        match event.button() {
            1 if crate::stack::is_folder(&id) => {
                let id = id.clone();
                let proxy = proxy.clone();
                let trigger = event.clone();
                glib::spawn_future_local(async move {
                    let entries = proxy.list_folder(&id).await.unwrap_or_default();
                    crate::stack::menu(&entries, proxy.clone())
                        .popup_at_pointer(Some(&trigger));
                });
            }
            1 => {
                let id = id.clone();
                let proxy = proxy.clone();
                glib::spawn_future_local(async move {
                    let _ = proxy.activate_item(&id).await;
                });
            }
            3 => {
                let item = menu_item.clone();
                let proxy = menu_proxy.clone();
                let trigger = event.clone();
                glib::spawn_future_local(async move {
                    let windows = proxy.item_windows(&item.id).await.unwrap_or_default();
                    context_menu(&item, &windows, proxy.clone())
                        .popup_at_pointer(Some(&trigger));
                });
            }
            _ => {}
        }
        glib::Propagation::Stop
    });

    accept_file_drops(&button, item, proxy_for_drop);

    ItemWidgets {
        root: button.upcast(),
        image,
    }
}

fn accept_file_drops(
    button: &gtk::EventBox,
    item: &DockItem,
    proxy: Rc<PrimoDockProxy<'static>>,
) {
    const URI_LIST: u32 = 0;
    let targets = [gtk::TargetEntry::new(
        "text/uri-list",
        gtk::TargetFlags::OTHER_APP,
        URI_LIST,
    )];
    button.drag_dest_set(gtk::DestDefaults::ALL, &targets, gdk::DragAction::COPY);

    let id = item.id.clone();
    button.connect_drag_data_received(move |_, _, _, _, data, _, _| {
        let paths: Vec<String> = data
            .uris()
            .iter()
            .filter_map(|uri| glib::filename_from_uri(uri).ok())
            .map(|(path, _)| path.to_string_lossy().into_owned())
            .collect();
        if paths.is_empty() {
            return;
        }
        let id = id.clone();
        let proxy = proxy.clone();
        glib::spawn_future_local(async move {
            let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
            if let Err(e) = proxy.open_with(&id, &refs).await {
                tracing::warn!("{id} could not open the dropped files: {e}");
            }
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: (i32, i32) = (1920, 1080);

    fn item(windows: Vec<u32>, active: bool) -> DockItem {
        DockItem {
            id: "app".into(),
            name: "App".into(),
            icon: "app".into(),
            pinned: true,
            windows,
            active,
        }
    }

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

    #[test]
    fn a_pinned_app_with_nothing_running_shows_no_indicator() {
        assert_eq!(indicator_state(&item(vec![], false)), "indicator-idle");
    }

    #[test]
    fn a_running_app_shows_an_indicator() {
        assert_eq!(indicator_state(&item(vec![1], false)), "indicator");
    }

    #[test]
    fn the_focused_app_is_told_apart_from_the_merely_running_ones() {
        assert_ne!(
            indicator_state(&item(vec![1], true)),
            indicator_state(&item(vec![1], false))
        );
    }

    fn write_square_png(side: i32) -> String {
        let path = std::env::temp_dir().join(format!("primodock-icon-{side}.png"));
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
        assert_eq!(icon_size_for(4, 0, SCREEN.0, 32), 32);
    }

    #[test]
    fn a_preferred_size_never_overrides_the_need_to_fit() {
        let crowded = icon_size_for(40, 4, SCREEN.0, ICON_SIZE);

        assert!(crowded < ICON_SIZE);
    }

    #[test]
    fn a_single_environment_needs_no_switcher() {
        assert!(!switcher_is_useful(0));
        assert!(!switcher_is_useful(1));
    }

    #[test]
    fn two_environments_are_worth_a_switcher() {
        assert!(switcher_is_useful(2));
    }

    #[test]
    fn a_handful_of_apps_keeps_icons_at_full_size() {
        assert_eq!(icon_size_for(6, 2, SCREEN.0, ICON_SIZE), ICON_SIZE);
    }

    #[test]
    fn twenty_eight_pinned_apps_shrink_the_icons_instead_of_overflowing() {
        let size = icon_size_for(28, 4, SCREEN.0, ICON_SIZE);

        assert!(size < ICON_SIZE);
        assert!(size >= MIN_ICON_SIZE);
    }

    #[test]
    fn the_shrunken_icons_actually_fit_the_screen_they_were_sized_for() {
        for count in 1..60 {
            let size = icon_size_for(count, 4, SCREEN.0, ICON_SIZE);
            let reserved = SWITCHER_WIDTH
                + ITEM_SPACING * 2
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
        assert_eq!(icon_size_for(500, 4, SCREEN.0, ICON_SIZE), MIN_ICON_SIZE);
    }

    #[test]
    fn an_empty_dock_does_not_divide_by_zero_sizing_its_icons() {
        assert_eq!(icon_size_for(0, 0, SCREEN.0, ICON_SIZE), ICON_SIZE);
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
        let twenty_eight = (28 * slot + SWITCHER_WIDTH, MIN_BAR_HEIGHT);

        let (width, _) = clamp_to_screen(twenty_eight, SCREEN);

        assert!(width <= SCREEN.0 - SCREEN_MARGIN);
    }

    #[test]
    fn an_empty_bar_is_never_clamped_to_nothing() {
        let (width, height) = clamp_to_screen((0, 0), SCREEN);

        assert_eq!(width, SWITCHER_WIDTH);
        assert_eq!(height, MIN_BAR_HEIGHT);
    }

    #[test]
    fn a_very_short_screen_still_leaves_room_for_one_row_of_icons() {
        let (_, height) = clamp_to_screen((900, 4000), (800, 200));

        assert_eq!(height, MIN_BAR_HEIGHT);
    }
}
