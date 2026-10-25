//! A pin pulled off the bar, carried at the pointer and let go of in a puff.
//! #18, P3: "dragging out unpins, with the poof".
//!
//! The row is one drawing inside the dock's window, and the window ends a
//! little above the bar: an icon dragged up off it would be cut off at the
//! window's top edge. So once it is off the bar it is drawn by a window of
//! its own that follows the pointer — the same thing GTK's own drag icon is,
//! without a drag protocol behind it, because nothing outside the dock is
//! being offered anything.
//!
//! The puff is drawn, not a picture: a ring of soft discs that grow apart
//! and fade, over `motion::POOF`. Grey and white so it reads over a dark
//! desktop and a light one alike, and never drawn at all with the desktop's
//! "Reduce animation" on, or with no compositor to make its air clear — the
//! icon simply goes.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use gtk::cairo;
use gtk::gdk_pixbuf;
use gtk::prelude::*;

/// The window an icon is carried in, off the bar.
#[derive(Clone)]
pub struct Float {
    window: gtk::Window,
    picture: Rc<std::cell::RefCell<Option<gdk_pixbuf::Pixbuf>>>,
}

/// A window that takes no part in anything: no input, no focus, no taskbar.
fn bare(kind: gdk::WindowTypeHint) -> gtk::Window {
    let window = gtk::Window::new(gtk::WindowType::Popup);
    window.set_type_hint(kind);
    window.set_app_paintable(true);
    window.set_decorated(false);
    window.set_accept_focus(false);
    if let Some(visual) = gtk::prelude::WidgetExt::screen(&window).and_then(|s| s.rgba_visual()) {
        window.set_visual(Some(&visual));
    }
    // It sits under the pointer, so it must not be what the pointer is over.
    window.connect_realize(|window| {
        let nothing = cairo::Region::create();
        window.input_shape_combine_region(Some(&nothing));
    });
    window
}

/// Clear a window to nothing, so what is drawn next is drawn on air.
fn clear(cr: &cairo::Context) {
    cr.set_operator(cairo::Operator::Source);
    cr.set_source_rgba(0.0, 0.0, 0.0, 0.0);
    let _ = cr.paint();
    cr.set_operator(cairo::Operator::Over);
}

impl Float {
    pub fn new() -> Self {
        let window = bare(gdk::WindowTypeHint::Dnd);
        let picture: Rc<std::cell::RefCell<Option<gdk_pixbuf::Pixbuf>>> = Rc::default();
        let drawing = picture.clone();
        window.connect_draw(move |_, cr| {
            clear(cr);
            if let Some(picture) = drawing.borrow().as_ref() {
                cr.set_source_pixbuf(picture, 0.0, 0.0);
                let _ = cr.paint();
            }
            glib::Propagation::Stop
        });
        Self { window, picture }
    }

    /// Carry this picture with its centre at `at`, in root coordinates.
    pub fn carry(&self, picture: gdk_pixbuf::Pixbuf, at: (f64, f64)) {
        let size = (picture.width(), picture.height());
        self.picture.replace(Some(picture));
        self.window.set_size_request(size.0, size.1);
        self.window.resize(size.0, size.1);
        self.follow(at);
        if !self.window.is_visible() {
            self.window.show();
        }
        self.window.queue_draw();
    }

    /// Keep the picture centred on the pointer.
    pub fn follow(&self, at: (f64, f64)) {
        let Some(picture) = self.picture.borrow().clone() else {
            return;
        };
        self.window.move_(
            (at.0 - picture.width() as f64 / 2.0).round() as i32,
            (at.1 - picture.height() as f64 / 2.0).round() as i32,
        );
    }

    pub fn put_down(&self) {
        self.window.hide();
        self.picture.replace(None);
    }

    pub fn carrying(&self) -> bool {
        self.window.is_visible()
    }

    #[cfg(test)]
    pub fn window(&self) -> &gtk::Window {
        &self.window
    }
}

/// How many discs the puff's ring is made of.
const PUFFS: usize = 7;

/// One disc of the puff: where it is from the puff's centre, how big, and
/// how opaque.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Puff {
    pub dx: f64,
    pub dy: f64,
    pub radius: f64,
    pub alpha: f64,
}

/// The puff `progress` of the way through, for an icon `size` across: the
/// discs start bunched over where the icon was, about its size, and drift
/// apart as they swell and fade — a little upward, the way smoke goes.
pub fn puffs(progress: f64, size: f64) -> Vec<Puff> {
    let t = crate::motion::ease_out(progress);
    let fade = 1.0 - progress.clamp(0.0, 1.0);
    let alpha = 0.85 * fade * fade;
    let rise = size * 0.2 * t;
    // One in the middle as well as the ring, or the puff is a ring with a
    // hole where the icon was — seen in the sandbox, and it read as a
    // doughnut rather than smoke.
    let middle = Puff {
        dx: 0.0,
        dy: -rise,
        radius: size * (0.26 + 0.12 * t),
        alpha,
    };
    std::iter::once(middle)
        .chain((0..PUFFS).map(|index| {
            let angle = std::f64::consts::TAU * index as f64 / PUFFS as f64;
            let reach = size * (0.15 + 0.45 * t);
            Puff {
                dx: angle.cos() * reach,
                dy: angle.sin() * reach * 0.8 - rise,
                radius: size * (0.18 + 0.22 * t),
                alpha,
            }
        }))
        .collect()
}

/// A puff at `at`, in root coordinates, where an icon `size` across was let
/// go of. It draws itself out and is gone; nothing waits on it.
///
/// Not drawn without a compositor either: the puff is mostly air, and with
/// nothing to blend it against an X server paints that air black — seen in
/// Xvfb as a dark square with smoke in it, which is worse than no smoke.
pub fn poof(at: (f64, f64), size: f64) {
    if crate::motion::still() {
        return;
    }
    if !gdk::Screen::default().is_some_and(|screen| screen.is_composited()) {
        return;
    }
    let span = (size * 2.4).ceil() as i32;
    let window = bare(gdk::WindowTypeHint::Notification);
    window.set_size_request(span, span);
    window.move_(
        at.0.round() as i32 - span / 2,
        at.1.round() as i32 - span / 2,
    );
    let started = Rc::new(Cell::new(Instant::now()));
    let drawing = started.clone();
    window.connect_draw(move |_, cr| {
        clear(cr);
        let progress = drawing.get().elapsed().as_secs_f64() / crate::motion::POOF.as_secs_f64();
        let centre = span as f64 / 2.0;
        for puff in puffs(progress, size) {
            let (x, y) = (centre + puff.dx, centre + puff.dy);
            let shade = cairo::RadialGradient::new(x, y, 0.0, x, y, puff.radius);
            shade.add_color_stop_rgba(0.0, 0.97, 0.97, 0.97, puff.alpha);
            shade.add_color_stop_rgba(0.7, 0.82, 0.82, 0.84, puff.alpha * 0.8);
            shade.add_color_stop_rgba(1.0, 0.7, 0.7, 0.72, 0.0);
            let _ = cr.set_source(&shade);
            cr.arc(x, y, puff.radius, 0.0, std::f64::consts::TAU);
            let _ = cr.fill();
        }
        glib::Propagation::Stop
    });
    window.show();
    started.set(Instant::now());
    window.add_tick_callback(move |window, _| {
        if started.get().elapsed() >= crate::motion::POOF {
            window.close();
            return glib::ControlFlow::Break;
        }
        window.queue_draw();
        glib::ControlFlow::Continue
    });
}

#[cfg(test)]
pub mod tests {
    use super::*;

    #[test]
    fn a_puff_starts_where_the_icon_was_and_spreads_out() {
        let first = puffs(0.0, 48.0);
        let last = puffs(1.0, 48.0);
        let spread = |puffs: &[Puff]| puffs.iter().map(|p| p.dx.hypot(p.dy)).fold(0.0, f64::max);
        assert!(
            spread(&first) < 48.0 / 2.0,
            "it did not start over the icon"
        );
        assert!(spread(&last) > spread(&first), "it never spread");
        assert!(last[0].radius > first[0].radius, "the discs did not swell");
    }

    #[test]
    fn a_puff_fades_to_nothing() {
        let alphas: Vec<f64> = (0..=10)
            .map(|step| puffs(step as f64 / 10.0, 48.0)[0].alpha)
            .collect();
        assert!(
            alphas.windows(2).all(|pair| pair[1] <= pair[0]),
            "{alphas:?}"
        );
        assert_eq!(*alphas.last().unwrap(), 0.0, "the last frame still shows");
        assert!(alphas[0] > 0.5, "the first frame is too faint to see");
    }

    #[test]
    fn a_puff_drifts_up_the_way_smoke_does() {
        let rise = |progress: f64| -> f64 {
            let puffs = puffs(progress, 48.0);
            puffs.iter().map(|p| p.dy).sum::<f64>() / puffs.len() as f64
        };
        assert!(rise(1.0) < rise(0.0), "the puff sank");
    }

    /// The window that carries the icon is the size of the picture, centred
    /// on the pointer, and takes no input — it is under the pointer, and the
    /// row has to keep hearing it.
    pub fn the_carried_icon_is_centred_on_the_pointer_and_takes_no_input() {
        let float = Float::new();
        let picture = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, true, 8, 48, 48)
            .expect("a pixbuf");
        float.carry(picture, (300.0, 200.0));
        while gtk::events_pending() {
            gtk::main_iteration_do(false);
        }
        assert!(float.carrying());
        let (x, y) = float.window().position();
        assert_eq!((x, y), (276, 176), "not centred on the pointer");

        float.follow((500.0, 100.0));
        assert_eq!(float.window().position(), (476, 76));

        float.put_down();
        assert!(
            !float.carrying(),
            "the icon stayed in the air after it was let go of"
        );
    }
}
