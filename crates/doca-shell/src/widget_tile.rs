use std::rc::Rc;

use gtk::prelude::*;
use doca_ipc::{DocaProxy, WidgetState, NO_PROGRESS};

pub const TILE_WIDTH: i32 = 88;

pub struct WidgetTile {
    pub root: gtk::Widget,
    label: gtk::Label,
    detail: gtk::Label,
    progress: gtk::ProgressBar,
}

impl WidgetTile {
    pub fn new(state: &WidgetState, proxy: Rc<DocaProxy<'static>>) -> Self {
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
            let id = id.clone();
            let proxy = proxy.clone();
            glib::spawn_future_local(async move {
                if let Err(e) = proxy.invoke_widget(&id, action).await {
                    tracing::warn!("widget {id} rejected {action}: {e}");
                }
            });
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
