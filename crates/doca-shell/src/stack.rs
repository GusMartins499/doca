use std::rc::Rc;

use gtk::prelude::*;
use doca_ipc::{FolderEntry, DocaProxy};

pub const PREFIX: &str = "folder:";
pub const COLUMNS: i32 = 5;

pub fn is_folder(id: &str) -> bool {
    id.starts_with(PREFIX)
}
pub const ENTRY_ICON: i32 = 32;

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

pub fn menu(entries: &[FolderEntry], proxy: Rc<DocaProxy<'static>>) -> gtk::Menu {
    let menu = gtk::Menu::new();
    menu.set_widget_name("stack");

    if entries.is_empty() {
        let empty = gtk::MenuItem::with_label("empty");
        empty.set_sensitive(false);
        menu.append(&empty);
        menu.show_all();
        return menu;
    }

    let columns = columns_for(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let label = gtk::Label::new(Some(&entry.name));
        label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        label.set_max_width_chars(14);
        label.set_widget_name("stack-label");

        let column = gtk::Box::new(gtk::Orientation::Vertical, 4);
        column.add(&entry_icon(entry));
        column.add(&label);

        let cell = gtk::MenuItem::new();
        cell.add(&column);
        cell.set_tooltip_text(Some(&entry.name));

        let path = entry.path.clone();
        let proxy = proxy.clone();
        cell.connect_activate(move |_| {
            let path = path.clone();
            let proxy = proxy.clone();
            glib::spawn_future_local(async move {
                if let Err(e) = proxy.open_path(&path).await {
                    tracing::warn!("cannot open {path}: {e}");
                }
            });
        });

        let (left, top) = grid_position(index, columns);
        menu.attach(&cell, left as u32, left as u32 + 1, top as u32, top as u32 + 1);
    }

    menu.show_all();
    menu
}

#[cfg(test)]
mod tests {
    use super::*;

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
