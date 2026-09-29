use std::rc::Rc;

use gtk::prelude::*;
use primodock_ipc::{DockItem, PrimoDockProxy};

pub const ICON_SIZE: i32 = 48;
pub const ITEM_SPACING: i32 = 6;
pub const ITEM_PADDING: i32 = 8;
pub const BAR_PADDING: i32 = 10;

pub fn bar_width(item_count: i32) -> i32 {
    let slot = ICON_SIZE + ITEM_PADDING * 2;
    let items = item_count.max(1);
    items * slot + (items - 1) * ITEM_SPACING + BAR_PADDING * 2
}

pub fn bar_height() -> i32 {
    ICON_SIZE + ITEM_PADDING * 2 + BAR_PADDING * 2
}

fn icon_widget(name: &str) -> gtk::Image {
    let theme = gtk::IconTheme::default();
    let pixbuf = theme.and_then(|theme| {
        theme
            .load_icon(name, ICON_SIZE, gtk::IconLookupFlags::FORCE_SIZE)
            .ok()
            .flatten()
    });

    match pixbuf {
        Some(pixbuf) => gtk::Image::from_pixbuf(Some(&pixbuf)),
        None if std::path::Path::new(name).is_absolute() => gtk::Image::from_file(name),
        None => gtk::Image::from_icon_name(
            Some("application-x-executable"),
            gtk::IconSize::Dialog,
        ),
    }
}

fn context_menu(item: &DockItem, proxy: Rc<PrimoDockProxy<'static>>) -> gtk::Menu {
    let menu = gtk::Menu::new();

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

pub fn item_button(item: &DockItem, proxy: Rc<PrimoDockProxy<'static>>) -> gtk::Widget {
    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
    column.add(&icon_widget(&item.icon));

    let indicator = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    indicator.set_widget_name(indicator_state(item));
    indicator.set_size_request(6, 3);
    indicator.set_halign(gtk::Align::Center);
    indicator.set_margin_top(3);
    column.add(&indicator);

    let button = gtk::EventBox::new();
    button.set_widget_name("item");
    button.set_tooltip_text(Some(&item.name));
    button.add(&column);

    let id = item.id.clone();
    let menu_item = item.clone();
    let menu_proxy = proxy.clone();
    button.connect_button_press_event(move |_, event| {
        match event.button() {
            1 => {
                let id = id.clone();
                let proxy = proxy.clone();
                glib::spawn_future_local(async move {
                    let _ = proxy.activate_item(&id).await;
                });
            }
            3 => {
                let menu = context_menu(&menu_item, menu_proxy.clone());
                menu.popup_at_pointer(Some(event));
            }
            _ => {}
        }
        glib::Propagation::Stop
    });

    button.upcast()
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn an_empty_dock_still_reserves_the_width_of_one_slot() {
        assert_eq!(bar_width(0), bar_width(1));
    }

    #[test]
    fn the_bar_grows_by_one_slot_and_one_gap_per_item() {
        let slot = ICON_SIZE + ITEM_PADDING * 2;

        assert_eq!(bar_width(3) - bar_width(2), slot + ITEM_SPACING);
    }

    #[test]
    fn the_bar_is_tall_enough_for_an_icon_and_its_padding() {
        assert!(bar_height() >= ICON_SIZE + ITEM_PADDING * 2);
    }
}
