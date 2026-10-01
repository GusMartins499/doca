//! A dictionary of changed keys, folded onto the settings already in place.
//!
//! `SetAppearance` takes `a{sv}` rather than a whole struct, so the work of
//! turning "whatever arrived on the bus" into a value the config will accept
//! happens here — once, in one place, testable without a bus. The daemon is
//! the only validator either way: `Appearance::sanitised` still has the last
//! word on every number that comes through.

use anyhow::{bail, Context, Result};
use zbus::zvariant::Value;

use crate::config::{Appearance, Setting};

/// Fold the named keys onto an appearance, leaving the rest untouched.
///
/// An unknown key is an error rather than a shrug: a caller that misspells
/// `icon_size` should hear about it, not watch the dock ignore it.
pub fn appearance<'a>(
    base: &Appearance,
    changes: impl IntoIterator<Item = (&'a str, &'a Value<'a>)>,
) -> Result<Appearance> {
    use doca_ipc::appearance_key as key;

    let mut patched = base.clone();
    let mut named = 0usize;

    for (name, value) in changes {
        match name {
            key::THEME => patched.theme = text(value).context("theme")?,
            key::ICON_SIZE => patched.icon_size = integer(value).context("icon_size")?,
            key::MAGNIFICATION => {
                patched.magnification = number(value).context("magnification")?
            }
            key::AUTO_HIDE => patched.auto_hide = flag(value).context("auto_hide")?,
            key::SHOW_TRASH => patched.show_trash = flag(value).context("show_trash")?,
            key::ICON_THEME => patched.icon_theme = text(value).context("icon_theme")?,
            key::GTK_THEME => patched.gtk_theme = text(value).context("gtk_theme")?,
            key::CURSOR_THEME => patched.cursor_theme = text(value).context("cursor_theme")?,
            unknown => bail!(
                "no appearance key {unknown}; known keys are {}",
                key::ALL.join(", ")
            ),
        }
        named += 1;
    }

    if named == 0 {
        bail!("nothing to change");
    }
    Ok(patched)
}

/// One widget setting, in whichever of the two shapes it arrived as.
///
/// Which shape a key actually wants is the config's business
/// (`WidgetSettings::set`); all this decides is whether the bytes on the wire
/// were a word or a number.
pub fn setting(value: &Value<'_>) -> Result<Setting> {
    match value {
        Value::Str(text) => Ok(Setting::Text(text.to_string())),
        Value::Bool(_) => bail!("a widget setting is text or a whole number, not a flag"),
        _ => Ok(Setting::Count(count(value)?)),
    }
}

fn text(value: &Value<'_>) -> Result<String> {
    match value {
        Value::Str(text) => Ok(text.to_string()),
        other => bail!("expected text, got {}", other.value_signature()),
    }
}

fn flag(value: &Value<'_>) -> Result<bool> {
    match value {
        Value::Bool(flag) => Ok(*flag),
        // `gdbus call` has no boolean literal of its own in every shell, and
        // a keybinding is a string anyway.
        Value::Str(text) => match text.as_str().trim().to_lowercase().as_str() {
            "true" | "1" | "on" | "yes" => Ok(true),
            "false" | "0" | "off" | "no" => Ok(false),
            other => bail!("{other:?} is not true or false"),
        },
        other => bail!("expected true or false, got {}", other.value_signature()),
    }
}

fn integer(value: &Value<'_>) -> Result<i32> {
    let whole = whole(value)?;
    i32::try_from(whole).with_context(|| format!("{whole} does not fit a size"))
}

fn count(value: &Value<'_>) -> Result<u32> {
    let whole = whole(value)?;
    u32::try_from(whole).with_context(|| format!("{whole} is not a count"))
}

/// Any integer the bus can carry, widened to one type before it is narrowed.
fn whole(value: &Value<'_>) -> Result<i64> {
    match value {
        Value::U8(n) => Ok(i64::from(*n)),
        Value::I16(n) => Ok(i64::from(*n)),
        Value::U16(n) => Ok(i64::from(*n)),
        Value::I32(n) => Ok(i64::from(*n)),
        Value::U32(n) => Ok(i64::from(*n)),
        Value::I64(n) => Ok(*n),
        Value::U64(n) => i64::try_from(*n).with_context(|| format!("{n} is too large")),
        Value::Str(text) => text
            .as_str()
            .trim()
            .parse()
            .with_context(|| format!("{:?} is not a whole number", text.as_str())),
        other => bail!("expected a whole number, got {}", other.value_signature()),
    }
}

fn number(value: &Value<'_>) -> Result<f64> {
    match value {
        Value::F64(n) => Ok(*n),
        Value::Str(text) => text
            .as_str()
            .trim()
            .parse()
            .with_context(|| format!("{:?} is not a number", text.as_str())),
        other => Ok(whole(other)? as f64),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use doca_ipc::appearance_key as key;

    fn base() -> Appearance {
        Appearance {
            theme: "native".to_string(),
            icon_size: 48,
            magnification: 1.6,
            show_trash: true,
            auto_hide: false,
            icon_theme: String::new(),
            gtk_theme: String::new(),
            cursor_theme: String::new(),
        }
    }

    fn patch<'a>(pairs: Vec<(&'a str, Value<'a>)>) -> Result<Appearance> {
        let base = base();
        let borrowed: Vec<(&str, &Value)> =
            pairs.iter().map(|(name, value)| (*name, value)).collect();
        appearance(&base, borrowed)
    }

    #[test]
    fn a_dictionary_of_one_key_changes_only_that_key() {
        let patched = patch(vec![(key::THEME, Value::from("paper"))]).unwrap();

        assert_eq!(patched.theme, "paper");
        assert_eq!(patched.icon_size, base().icon_size, "the rest is left alone");
        assert_eq!(patched.magnification, base().magnification);
        assert_eq!(patched.auto_hide, base().auto_hide);
        assert_eq!(patched.show_trash, base().show_trash);
    }

    #[test]
    fn every_key_the_contract_names_can_actually_be_set() {
        let patched = patch(vec![
            (key::THEME, Value::from("midnight")),
            (key::ICON_SIZE, Value::from(64i32)),
            (key::MAGNIFICATION, Value::from(2.0f64)),
            (key::AUTO_HIDE, Value::from(true)),
            (key::SHOW_TRASH, Value::from(false)),
        ])
        .unwrap();

        assert_eq!(patched.theme, "midnight");
        assert_eq!(patched.icon_size, 64);
        assert_eq!(patched.magnification, 2.0);
        assert!(patched.auto_hide);
        assert!(!patched.show_trash);
    }

    #[test]
    fn a_theme_of_the_system_can_be_overridden_for_the_dock_alone() {
        let patched = patch(vec![
            (key::ICON_THEME, Value::from("Papirus-Dark")),
            (key::GTK_THEME, Value::from("Adwaita-dark")),
            (key::CURSOR_THEME, Value::from("McMojave-cursors")),
        ])
        .unwrap();

        assert_eq!(patched.icon_theme, "Papirus-Dark");
        assert_eq!(patched.gtk_theme, "Adwaita-dark");
        assert_eq!(patched.cursor_theme, "McMojave-cursors");
    }

    #[test]
    fn an_override_is_given_back_to_the_system_with_an_empty_name() {
        let mut base = base();
        base.icon_theme = "Papirus-Dark".to_string();
        let given_back =
            appearance(&base, vec![(key::ICON_THEME, &Value::from(""))]).unwrap();

        assert!(
            given_back.icon_theme.is_empty(),
            "there has to be a way back to whatever GNOME Tweaks says"
        );
    }

    #[test]
    fn a_misspelled_key_is_refused_rather_than_ignored() {
        let refused = patch(vec![("icon-size", Value::from(64i32))]);

        assert!(refused.is_err(), "a key nobody knows must not pass quietly");
    }

    #[test]
    fn an_empty_dictionary_is_not_a_write() {
        assert!(patch(vec![]).is_err());
    }

    #[test]
    fn a_value_of_the_wrong_shape_is_refused() {
        assert!(patch(vec![(key::ICON_SIZE, Value::from("huge"))]).is_err());
        assert!(patch(vec![(key::THEME, Value::from(3i32))]).is_err());
        assert!(patch(vec![(key::AUTO_HIDE, Value::from(0.5f64))]).is_err());
    }

    #[test]
    fn a_number_typed_by_hand_on_the_command_line_still_arrives() {
        // `gdbus call` and a keybinding both send strings.
        let patched = patch(vec![
            (key::ICON_SIZE, Value::from("64")),
            (key::MAGNIFICATION, Value::from("1.8")),
            (key::AUTO_HIDE, Value::from("yes")),
        ])
        .unwrap();

        assert_eq!(patched.icon_size, 64);
        assert_eq!(patched.magnification, 1.8);
        assert!(patched.auto_hide);
    }

    #[test]
    fn an_integer_of_any_width_is_read_as_a_size() {
        for value in [
            Value::from(64u8),
            Value::from(64i16),
            Value::from(64u16),
            Value::from(64u32),
            Value::from(64i64),
            Value::from(64u64),
        ] {
            assert_eq!(
                patch(vec![(key::ICON_SIZE, value)]).unwrap().icon_size,
                64,
                "a caller should not have to guess the daemon's integer width"
            );
        }
    }

    #[test]
    fn a_whole_number_is_accepted_where_a_fraction_is_expected() {
        assert_eq!(
            patch(vec![(key::MAGNIFICATION, Value::from(2i32))])
                .unwrap()
                .magnification,
            2.0
        );
    }

    #[test]
    fn a_widget_setting_arrives_as_text_or_as_a_count() {
        assert_eq!(
            setting(&Value::from("2026-12-25")).unwrap(),
            Setting::Text("2026-12-25".to_string())
        );
        assert_eq!(setting(&Value::from(25u32)).unwrap(), Setting::Count(25));
    }

    #[test]
    fn a_flag_is_not_a_widget_setting() {
        assert!(setting(&Value::from(true)).is_err());
    }
}
