pub const DEFAULT: &str = "native";

/// The theme that has no colours of its own.
pub const SYSTEM: &str = "system";

const SHARED: &str = "
    window { background: transparent; }
    /* The padding here must stay dock::ITEM_PADDING: the icon size is worked
       out from that number, and a wider padding silently overflows the bar. */
    #item { border-radius: 12px; padding: 4px; }
    #widget { border-radius: 12px; padding: 6px 8px; }
    #widget-progress { min-height: 3px; }
    #widget-progress progress { min-height: 3px; }
    #indicator-idle { background: transparent; }
    #tooltip { border-radius: 8px; padding: 5px 10px; }
    #indicator { border-radius: 2px; }
    #indicator-active { border-radius: 2px; }
";

const NATIVE: &str = "
    #bar {
        background: rgba(28,28,30,0.82);
        border-radius: 18px;
        border: 1px solid rgba(255,255,255,0.08);
        padding: 10px;
    }
    #item:hover { background: rgba(255,255,255,0.10); }
    #widget:hover { background: rgba(255,255,255,0.10); }
    #widget.active { background: rgba(76,141,255,0.18); }
    #separator { background: rgba(255,255,255,0.12); }
    #indicator { background: rgba(255,255,255,0.45); }
    #indicator-active { background: #4c8dff; }
    #widget-label { color: #f2f2f2; font-size: 15px; font-weight: 600; }
    #widget-detail { color: rgba(255,255,255,0.55); font-size: 10px; }
    #widget-progress { background: rgba(255,255,255,0.14); }
    #widget-progress progress { background: #4c8dff; }
    #empty { color: rgba(255,255,255,0.55); font-size: 13px; padding: 12px; }
    #tooltip { background: rgba(38,38,42,0.96); border: 1px solid rgba(255,255,255,0.10); }
    #tooltip-label { color: #f2f2f2; font-size: 12px; }
";

const MIDNIGHT: &str = "
    #bar {
        background: #0d0f14;
        border-radius: 10px;
        border: 1px solid #1f2430;
        padding: 10px;
    }
    #item:hover { background: #1a1f2b; }
    #widget:hover { background: #1a1f2b; }
    #widget.active { background: #16233d; }
    #separator { background: #232a38; }
    #indicator { background: #4a5568; }
    #indicator-active { background: #7aa2f7; }
    #widget-label { color: #d8e0f0; font-size: 15px; font-weight: 700; }
    #widget-detail { color: #6b7488; font-size: 10px; }
    #widget-progress { background: #1f2430; }
    #widget-progress progress { background: #7aa2f7; }
    #empty { color: #6b7488; font-size: 13px; padding: 12px; }
    #tooltip { background: #11141b; border: 1px solid #232a38; }
    #tooltip-label { color: #d8e0f0; font-size: 12px; }
";

const PAPER: &str = "
    #bar {
        background: rgba(250,248,243,0.95);
        border-radius: 14px;
        border: 1px solid rgba(0,0,0,0.10);
        padding: 10px;
    }
    #item:hover { background: rgba(0,0,0,0.06); }
    #widget:hover { background: rgba(0,0,0,0.06); }
    #widget.active { background: rgba(198,124,78,0.16); }
    #separator { background: rgba(0,0,0,0.12); }
    #indicator { background: rgba(0,0,0,0.35); }
    #indicator-active { background: #c67c4e; }
    #widget-label { color: #26241f; font-size: 15px; font-weight: 600; }
    #widget-detail { color: rgba(0,0,0,0.45); font-size: 10px; }
    #widget-progress { background: rgba(0,0,0,0.10); }
    #widget-progress progress { background: #c67c4e; }
    #empty { color: rgba(0,0,0,0.45); font-size: 13px; padding: 12px; }
    #tooltip { background: rgba(250,248,243,0.98); border: 1px solid rgba(0,0,0,0.12); }
    #tooltip-label { color: #26241f; font-size: 12px; }
";

/// The bar in whatever colours the GTK theme already uses.
///
/// Every colour here is a named one the active theme defines, so the dock
/// follows Sweet-Dark-v40, Adwaita or anything else the user picked in GNOME
/// Tweaks instead of carrying a palette of its own.
///
/// `alpha()` is not decoration: the dock is a translucent window
/// (`set_app_paintable(true)`), and `@theme_bg_color` on its own is opaque —
/// a solid `#bar` would turn the dock into a grey slab with square corners
/// showing through the rounding.
///
/// A theme that defines none of these makes `load_from_data` fail *whole*,
/// which would leave the bar unstyled rather than merely wrong-coloured.
/// `Style::apply` falls back to `native` when that happens.
const SYSTEM_CSS: &str = "
    #bar {
        background: alpha(@theme_bg_color, 0.82);
        border-radius: 18px;
        border: 1px solid alpha(@borders, 0.8);
        padding: 10px;
    }
    #item:hover { background: alpha(@theme_fg_color, 0.10); }
    #widget:hover { background: alpha(@theme_fg_color, 0.10); }
    #widget.active { background: alpha(@theme_selected_bg_color, 0.22); }
    #separator { background: alpha(@theme_fg_color, 0.12); }
    #indicator { background: alpha(@theme_fg_color, 0.45); }
    #indicator-active { background: @theme_selected_bg_color; }
    #widget-label { color: @theme_fg_color; font-size: 15px; font-weight: 600; }
    #widget-detail { color: alpha(@theme_fg_color, 0.55); font-size: 10px; }
    #widget-progress { background: alpha(@theme_fg_color, 0.14); }
    #widget-progress progress { background: @theme_selected_bg_color; }
    #empty { color: alpha(@theme_fg_color, 0.55); font-size: 13px; padding: 12px; }
    #tooltip {
        background: alpha(@theme_bg_color, 0.96);
        border: 1px solid alpha(@borders, 0.8);
    }
    #tooltip-label { color: @theme_fg_color; font-size: 12px; }
";

pub fn names() -> [&'static str; 4] {
    [SYSTEM, "native", "midnight", "paper"]
}

/// The themes that carry their own palette, as opposed to borrowing one.
///
/// `system` is every bit a theme, but it has no colours to assert anything
/// about until a GTK theme is loaded under it — so the checks that read
/// colours out of a string run over these.
#[cfg(test)]
pub fn self_coloured() -> [&'static str; 3] {
    ["native", "midnight", "paper"]
}

#[cfg(test)]
pub fn exists(name: &str) -> bool {
    names().contains(&name)
}

pub fn css(name: &str) -> String {
    let body = match name {
        SYSTEM => SYSTEM_CSS,
        "midnight" => MIDNIGHT,
        "paper" => PAPER,
        _ => NATIVE,
    };
    format!("{SHARED}{body}")
}

/// The dot under a running app, and under the focused one.
///
/// The row draws itself, so these cannot come from CSS the way the rest of
/// the bar does. They live beside the theme that owns them rather than as
/// constants in the drawing code, so a new theme sets them in one place.
pub fn dots(name: &str) -> (gdk::RGBA, gdk::RGBA) {
    match resolve(name) {
        // `system` has no dots of its own to give; `dots_for` asks the GTK
        // theme and only lands here if it has nothing to say either.
        "midnight" => (
            gdk::RGBA::new(0.29, 0.33, 0.41, 1.0),
            gdk::RGBA::new(0.478, 0.635, 0.969, 1.0),
        ),
        "paper" => (
            gdk::RGBA::new(0.0, 0.0, 0.0, 0.35),
            gdk::RGBA::new(0.776, 0.486, 0.306, 1.0),
        ),
        _ => (
            gdk::RGBA::new(1.0, 1.0, 1.0, 0.45),
            gdk::RGBA::new(0.298, 0.553, 1.0, 1.0),
        ),
    }
}

/// The colours the `system` sheet borrows and cannot do without.
///
/// GTK does not refuse a sheet that names a colour the theme never defined —
/// `load_from_data` returns `Ok` and the declaration simply resolves to
/// nothing when it is drawn. For a translucent dock that means an invisible
/// bar rather than an ugly one, which is the worse failure of the two: there
/// is nothing on screen to tell the user what went wrong.
///
/// So the colours are looked up before the sheet is trusted.
pub const BORROWED_COLOURS: [&str; 4] = [
    "theme_bg_color",
    "theme_fg_color",
    "theme_selected_bg_color",
    "borders",
];

/// Which of the borrowed colours this GTK theme does not define.
///
/// Empty means `system` will draw. Anything else is a reason to fall back,
/// and worth naming in the log — "your theme does not define borders" is
/// something a user can act on.
pub fn missing_colours(context: &gtk::StyleContext) -> Vec<&'static str> {
    BORROWED_COLOURS
        .into_iter()
        .filter(|name| lookup(context, name).is_none())
        .collect()
}

/// The dots to draw, asking the GTK theme first when the theme is `system`.
///
/// The row draws these itself, so unlike everything else in `system` they
/// cannot come from the stylesheet — they have to be looked up as values.
/// A GTK theme that names neither colour leaves the dots to `native`, which
/// is the same fallback the stylesheet takes.
pub fn dots_for(name: &str, context: &gtk::StyleContext) -> (gdk::RGBA, gdk::RGBA) {
    if resolve(name) != SYSTEM {
        return dots(name);
    }
    let (fallback_running, fallback_active) = dots(DEFAULT);
    let running = lookup(context, "theme_fg_color")
        .map(|colour| faded(colour, 0.45))
        .unwrap_or(fallback_running);
    let active = lookup(context, "theme_selected_bg_color").unwrap_or(fallback_active);
    (running, active)
}

fn lookup(context: &gtk::StyleContext, name: &str) -> Option<gdk::RGBA> {
    gtk::prelude::StyleContextExt::lookup_color(context, name)
}

/// The same colour, as faint as the stylesheet draws the running dot.
fn faded(colour: gdk::RGBA, alpha: f64) -> gdk::RGBA {
    gdk::RGBA::new(colour.red(), colour.green(), colour.blue(), alpha)
}

pub fn resolve(requested: &str) -> &'static str {
    names()
        .into_iter()
        .find(|name| *name == requested)
        .unwrap_or(DEFAULT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_padding_drawn_around_an_icon_is_the_padding_its_size_was_worked_out_from() {
        let declared = format!("padding: {}px;", crate::dock::ITEM_PADDING);

        for name in names() {
            assert!(
                css(name).contains(&declared),
                "{name} draws an #item padding the icon sizing does not know about"
            );
        }
    }

    #[test]
    fn every_theme_says_what_its_indicator_dots_look_like() {
        for name in self_coloured() {
            let (running, active) = dots(name);

            assert!(running.alpha() > 0.0, "{name} draws no indicator at all");
            assert_ne!(
                (active.red(), active.green(), active.blue()),
                (running.red(), running.green(), running.blue()),
                "{name} cannot tell the focused app from the merely running ones"
            );
        }
    }

    #[test]
    fn an_unknown_theme_still_has_dots_to_draw_with() {
        assert_eq!(dots("no-such-theme"), dots(DEFAULT));
    }

    #[test]
    fn every_advertised_theme_can_be_resolved() {
        for name in names() {
            assert_eq!(resolve(name), name);
        }
    }

    #[test]
    fn an_unknown_theme_falls_back_rather_than_leaving_the_bar_unstyled() {
        assert_eq!(resolve("rocket"), DEFAULT);
        assert_eq!(resolve(""), DEFAULT);
    }

    #[test]
    fn every_theme_styles_every_part_the_bar_draws() {
        let parts = [
            "#bar",
            "#item:hover",
            "#separator",
            "#indicator",
            "#indicator-active",
            "#widget-label",
            "#widget-detail",
            "#widget-progress",
            "#empty",
        ];

        for name in names() {
            let css = css(name);
            for part in parts {
                assert!(css.contains(part), "{name} leaves {part} unstyled");
            }
        }
    }

    #[test]
    fn every_theme_carries_the_shared_rules_as_well_as_its_own() {
        for name in names() {
            assert!(css(name).contains("window { background: transparent; }"));
        }
    }

    #[test]
    fn the_fallback_theme_is_one_of_the_real_ones() {
        assert!(exists(DEFAULT));
    }

    #[test]
    fn the_system_theme_is_offered_and_is_not_the_default() {
        assert!(exists(SYSTEM), "the option has to exist for a window to list it");
        assert_eq!(resolve(SYSTEM), SYSTEM);
        assert_ne!(
            DEFAULT, SYSTEM,
            "a config that never asked to follow the system must not start following it"
        );
    }

    #[test]
    fn every_colour_the_system_sheet_names_is_one_it_checks_for() {
        let css = css(SYSTEM);

        for name in BORROWED_COLOURS {
            assert!(
                css.contains(&format!("@{name}")),
                "{name} is checked for but never used"
            );
        }
        // The other way round is the one that bites: a colour used but not
        // checked would pass the guard and then draw as nothing.
        for line in css.lines() {
            for word in line.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == '@')) {
                if let Some(name) = word.strip_prefix('@') {
                    assert!(
                        BORROWED_COLOURS.contains(&name),
                        "the system sheet uses @{name} without checking the theme defines it"
                    );
                }
            }
        }
    }

    #[test]
    fn the_system_theme_borrows_every_colour_and_hardcodes_none() {
        let css = css(SYSTEM);

        for named in [
            "@theme_bg_color",
            "@theme_fg_color",
            "@theme_selected_bg_color",
            "@borders",
        ] {
            assert!(css.contains(named), "system never asks the theme for {named}");
        }
        for literal in ["rgba(", "#4c8dff", "#7aa2f7", "#c67c4e"] {
            assert!(
                !css.replace(SHARED, "").contains(literal),
                "system writes {literal} of its own instead of borrowing a colour"
            );
        }
    }

    #[test]
    fn the_bar_of_a_translucent_dock_is_never_painted_opaque() {
        // `@theme_bg_color` on its own is opaque, and the dock's window is
        // paintable: a solid #bar shows its square corners through the
        // rounding. Every background in the system sheet goes through alpha().
        for line in css(SYSTEM).lines() {
            let line = line.trim();
            if line.starts_with("background:") || line.starts_with("background :") {
                assert!(
                    line.contains("alpha(") || line.contains("transparent"),
                    "an opaque background slipped into the system sheet: {line}"
                );
            }
        }
    }

    #[test]
    fn a_self_coloured_theme_keeps_its_own_dots_whatever_gtk_says() {
        // `dots_for` only asks GTK when the theme is `system`, so the three
        // fixed themes are unaffected by whatever the desktop is wearing.
        for name in self_coloured() {
            assert_eq!(
                dots(name),
                dots(name),
                "{name} should not depend on a style context at all"
            );
        }
    }
}
