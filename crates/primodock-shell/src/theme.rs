pub const DEFAULT: &str = "native";

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

pub fn names() -> [&'static str; 3] {
    ["native", "midnight", "paper"]
}

#[cfg(test)]
pub fn exists(name: &str) -> bool {
    names().contains(&name)
}

pub fn css(name: &str) -> String {
    let body = match name {
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
        for name in names() {
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
}
