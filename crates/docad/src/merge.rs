//! Writing the config back without throwing away what the person wrote in it.
//!
//! Serialising the whole `Config` and saving that is the obvious way, and it
//! is what this used to do. It costs the file: a comment the user put above
//! `[appearance]`, the order they chose for their pins, the blank line they
//! left between two docks — none of it is in the struct, so none of it
//! survives a round trip. And the config is the thing this project tells
//! people to edit by hand, so it is their file, not ours to reformat because
//! a widget counted a glass of water.
//!
//! So the write is a merge rather than a replacement, and the rule is one
//! line long: **a value that did not change is not touched at all.** Only the
//! keys whose values actually differ are written, and only those; everything
//! else — comments, spacing, key order, the quoting style someone chose — is
//! left exactly as it was found. A key that is gone from the struct is
//! removed, a key that is new is appended, and that is the whole of it.
//!
//! The cost, named: changing an *array* rewrites that array, so a comment
//! sitting among its entries goes with it. Keeping those would mean matching
//! entries across a rewrite, which cannot be done for a list of plain strings
//! without guessing. A comment above the key survives either way, because
//! that lives on the key and the key is never replaced.

use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, Value};

/// `fresh` written into `existing`, keeping everything `existing` says that
/// `fresh` has no opinion about.
///
/// Both are TOML. `existing` being empty — or not parsing, which is the file
/// somebody broke with an editor — makes this `fresh` unchanged: there is
/// nothing to keep, and a config that cannot be read is not one to merge into.
pub fn keeping(existing: &str, fresh: &str) -> String {
    let Ok(mut kept) = existing.parse::<DocumentMut>() else {
        return fresh.to_string();
    };
    let Ok(new) = fresh.parse::<DocumentMut>() else {
        // `fresh` comes from our own serialiser, so this is not a case that
        // happens — but returning the input beats panicking in a daemon.
        return fresh.to_string();
    };
    if existing.trim().is_empty() {
        return fresh.to_string();
    }

    table(kept.as_table_mut(), new.as_table());
    kept.to_string()
}

/// Fold one table onto another, key by key.
fn table(kept: &mut Table, fresh: &Table) {
    for (key, new) in fresh.iter() {
        match (kept.get_mut(key), new) {
            // Both tables: go down. Nothing at this level is rewritten, so
            // the header, its comment and its place in the file all stay.
            (Some(Item::Table(old)), Item::Table(new)) => table(old, new),
            (Some(Item::ArrayOfTables(old)), Item::ArrayOfTables(new)) => {
                docks(old, new)
            }
            (Some(Item::Value(old)), Item::Value(new)) => value(old, new),
            // A key that changed shape — a value where there was a table —
            // has no formatting worth keeping.
            (Some(old), new) => *old = new.clone(),
            (None, new) => {
                kept.insert(key, new.clone());
            }
        }
    }

    let gone: Vec<String> = kept
        .iter()
        .map(|(key, _)| key.to_string())
        .filter(|key| fresh.get(key).is_none())
        .collect();
    for key in gone {
        kept.remove(&key);
    }
}

/// Replace a value only if it really is a different value.
///
/// The decor is carried across rather than taken from the new value: it is
/// where a trailing `# comment` and the spacing around the `=` live, and
/// neither changed just because a number did.
fn value(kept: &mut Value, fresh: &Value) {
    if same(kept, fresh) {
        return;
    }
    let mut replacement = fresh.clone();
    *replacement.decor_mut() = kept.decor().clone();
    *kept = replacement;
}

/// `[[environments]]` and anything else shaped like it, matched by `name`.
///
/// By name and not by position, because the docks can be reordered and one
/// that moved has not changed — its comment should move with it rather than
/// be handed to whoever took its place. A dock that was *renamed* is a new
/// one as far as this can tell, and loses its comment; that is the one case
/// matching by name cannot see through, and it is rarer than reordering.
fn docks(kept: &mut toml_edit::ArrayOfTables, fresh: &toml_edit::ArrayOfTables) {
    // Where in the file these tables sit, in the order they sit. A table
    // carries its own line position, so rebuilding the array in a new order
    // is not enough to reorder the *document* — the places have to be handed
    // out again, in the new order, out of the same set of places. Which also
    // means the docks stay where they are as a block: reordering them does
    // not move them above `[appearance]` or below `[widgets]`.
    let mut places: Vec<isize> = kept.iter().filter_map(|entry| entry.position()).collect();
    places.sort_unstable();

    let mut merged = toml_edit::ArrayOfTables::new();
    for new in fresh.iter() {
        match named(kept, name_of(new)) {
            Some(mut old) => {
                table(&mut old, new);
                merged.push(old);
            }
            None => merged.push(new.clone()),
        }
    }
    for (entry, place) in merged.iter_mut().zip(places) {
        entry.set_position(Some(place));
    }

    *kept = merged;
}

fn name_of(entry: &Table) -> Option<&str> {
    entry.get("name")?.as_str()
}

fn named(entries: &toml_edit::ArrayOfTables, name: Option<&str>) -> Option<Table> {
    let name = name?;
    entries
        .iter()
        .find(|entry| name_of(entry) == Some(name))
        .cloned()
}

/// The same value, ignoring everything about how it is written down.
fn same(a: &Value, b: &Value) -> bool {
    bare(a) == bare(b)
}

fn bare(value: &Value) -> String {
    let mut value = value.clone();
    plain(&mut value);
    value.to_string()
}

/// Strip a value of its spacing and comments, so two can be compared on what
/// they say rather than on how they were typed.
fn plain(value: &mut Value) {
    value.decor_mut().set_prefix("");
    value.decor_mut().set_suffix("");
    if let Some(array) = value.as_array_mut() {
        plain_array(array);
    }
    if let Some(inline) = value.as_inline_table_mut() {
        plain_inline(inline);
    }
}

fn plain_array(array: &mut Array) {
    for item in array.iter_mut() {
        plain(item);
    }
    array.set_trailing("");
    array.set_trailing_comma(false);
}

fn plain_inline(inline: &mut InlineTable) {
    for (_, item) in inline.iter_mut() {
        plain(item);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The file as somebody who edits it by hand would leave it.
    const WRITTEN_BY_HAND: &str = r#"# My dock. Do not let the daemon eat this.
[appearance]
theme   = "midnight"   # dark, because the desk is dark
icon_size = 48

# The dock I use for work.
[[environments]]
name = "Work"
pinned = [
  "code",      # the one I live in
  "firefox",
]

[[environments]]
name = "Home"
pinned = ["spotify"]

[widgets.timer]
minutes = 25
"#;

    fn saved(existing: &str, fresh: &str) -> String {
        keeping(existing, fresh)
    }

    #[test]
    fn a_write_that_changes_nothing_leaves_the_file_byte_for_byte() {
        let same = saved(WRITTEN_BY_HAND, WRITTEN_BY_HAND);

        assert_eq!(same, WRITTEN_BY_HAND);
    }

    #[test]
    fn the_comments_outlive_a_value_that_changed() {
        let fresh = WRITTEN_BY_HAND.replace("icon_size = 48", "icon_size = 64");

        let written = saved(WRITTEN_BY_HAND, &fresh);

        assert!(written.contains("icon_size = 64"), "the change did not land");
        for kept in [
            "# My dock. Do not let the daemon eat this.",
            "# dark, because the desk is dark",
            "# The dock I use for work.",
            "# the one I live in",
        ] {
            assert!(written.contains(kept), "{kept} was eaten");
        }
    }

    /// The spacing someone lined up by hand is theirs, not ours.
    #[test]
    fn a_value_that_changed_keeps_the_spacing_around_it() {
        let fresh = WRITTEN_BY_HAND.replace(r#""midnight""#, r#""paper""#);

        let written = saved(WRITTEN_BY_HAND, &fresh);

        assert!(
            written.contains(r#"theme   = "paper"   # dark, because the desk is dark"#),
            "the line was reflowed:\n{written}"
        );
    }

    #[test]
    fn the_order_of_the_keys_is_the_order_they_were_written_in() {
        // Serialised fresh, the struct's own order would put `appearance`
        // after `widgets` and sort the keys inside it.
        let fresh = "[widgets.timer]\nminutes = 25\n\n[appearance]\nicon_size = 48\ntheme = \"midnight\"\n\n[[environments]]\nname = \"Work\"\npinned = [\"code\", \"firefox\"]\n\n[[environments]]\nname = \"Home\"\npinned = [\"spotify\"]\n";

        let written = saved(WRITTEN_BY_HAND, fresh);

        let appearance = written.find("[appearance]").expect("appearance");
        let widgets = written.find("[widgets.timer]").expect("widgets");
        assert!(appearance < widgets, "the file was reordered:\n{written}");
    }

    #[test]
    fn a_key_that_is_new_is_appended_rather_than_lost() {
        let fresh = WRITTEN_BY_HAND.replace(
            "icon_size = 48",
            "icon_size = 48\nauto_hide = true",
        );

        let written = saved(WRITTEN_BY_HAND, &fresh);

        assert!(written.contains("auto_hide = true"));
    }

    #[test]
    fn a_key_that_is_gone_from_the_struct_goes_from_the_file() {
        let fresh = WRITTEN_BY_HAND.replace("icon_size = 48\n", "");

        let written = saved(WRITTEN_BY_HAND, &fresh);

        assert!(!written.contains("icon_size"), "a dead key stayed:\n{written}");
        assert!(written.contains("theme"), "its neighbour went with it");
    }

    /// The case that made this worth doing at all: the water widget counting
    /// a glass must not cost somebody their file.
    #[test]
    fn one_number_deep_in_the_file_rewrites_one_line_and_no_others() {
        let fresh = WRITTEN_BY_HAND.replace("minutes = 25", "minutes = 30");

        let written = saved(WRITTEN_BY_HAND, &fresh);

        let before: Vec<&str> = WRITTEN_BY_HAND.lines().collect();
        let after: Vec<&str> = written.lines().collect();
        assert_eq!(before.len(), after.len(), "the file changed shape");
        let differing: Vec<usize> = (0..before.len())
            .filter(|at| before[*at] != after[*at])
            .collect();
        assert_eq!(differing.len(), 1, "more than one line moved: {differing:?}");
    }

    /// Reordering the docks must carry each one's comment with it, which is
    /// why they are matched by name rather than by position.
    #[test]
    fn a_dock_that_moved_takes_its_own_comment_with_it() {
        let fresh = "[appearance]\ntheme = \"midnight\"\nicon_size = 48\n\n[[environments]]\nname = \"Home\"\npinned = [\"spotify\"]\n\n[[environments]]\nname = \"Work\"\npinned = [\"code\", \"firefox\"]\n\n[widgets.timer]\nminutes = 25\n";

        let written = saved(WRITTEN_BY_HAND, fresh);

        let home = written.find("name = \"Home\"").expect("Home");
        let work = written.find("name = \"Work\"").expect("Work");
        assert!(home < work, "the reorder did not land:\n{written}");
        let comment = written.find("# The dock I use for work.").expect("the comment");
        assert!(
            comment < work && comment > home,
            "the comment stayed behind with the wrong dock:\n{written}"
        );
    }

    #[test]
    fn a_dock_that_is_new_arrives_whole() {
        let fresh = format!(
            "{WRITTEN_BY_HAND}\n[[environments]]\nname = \"Play\"\npinned = [\"steam\"]\n"
        );

        let written = saved(WRITTEN_BY_HAND, &fresh);

        assert!(written.contains("name = \"Play\""));
        assert!(written.contains("steam"));
    }

    #[test]
    fn a_dock_that_went_is_taken_out() {
        let fresh = WRITTEN_BY_HAND
            .replace("[[environments]]\nname = \"Home\"\npinned = [\"spotify\"]\n", "");

        let written = saved(WRITTEN_BY_HAND, &fresh);

        assert!(!written.contains("\"Home\""), "the dock stayed:\n{written}");
        assert!(written.contains("\"Work\""));
    }

    /// A file somebody broke with an editor is not one to merge into — the
    /// merge would have nothing to hold on to, and guessing at half a
    /// document is how a config gets mangled instead of replaced.
    #[test]
    fn a_file_that_does_not_parse_is_replaced_rather_than_guessed_at() {
        let fresh = "[appearance]\ntheme = \"native\"\n";

        assert_eq!(saved("[appearance\ntheme =", fresh), fresh);
    }

    #[test]
    fn a_file_that_is_not_there_yet_is_simply_written() {
        let fresh = "[appearance]\ntheme = \"native\"\n";

        assert_eq!(saved("", fresh), fresh);
        assert_eq!(saved("\n  \n", fresh), fresh);
    }

    /// The named cost, asserted rather than left as a claim: an array that
    /// changed is rewritten, and a comment among its entries goes with it.
    /// The comment above the key is on the key, so it stays.
    ///
    /// `fresh` is written the way the serialiser really writes it — no
    /// comments anywhere — because that is the only thing this ever merges.
    #[test]
    fn a_comment_inside_an_array_that_changed_goes_with_the_array() {
        let fresh = "[appearance]\ntheme = \"midnight\"\nicon_size = 48\n\n\
                     [[environments]]\nname = \"Work\"\n\
                     pinned = [\"code\", \"firefox\", \"mpv\"]\n\n\
                     [[environments]]\nname = \"Home\"\npinned = [\"spotify\"]\n\n\
                     [widgets.timer]\nminutes = 25\n";

        let written = saved(WRITTEN_BY_HAND, fresh);

        assert!(written.contains("mpv"), "the change did not land");
        assert!(
            !written.contains("# the one I live in"),
            "the cost is not what the module says it is:\n{written}"
        );
        assert!(
            written.contains("# The dock I use for work."),
            "the comment above the table went too, which is not the deal"
        );
    }

    /// The whole path the daemon really takes: a hand-written file, parsed
    /// into the struct, one field changed, serialised and merged back.
    ///
    /// The other tests here hand-write both sides, which is the only way to
    /// pin down a single behaviour — but it also means they would all go on
    /// passing if `Config` serialised to something this cannot merge. This is
    /// the one that would notice.
    #[test]
    fn a_real_config_round_trips_through_the_struct_without_losing_the_file() {
        let mut config: crate::config::Config =
            toml::from_str(WRITTEN_BY_HAND).expect("the sample parses");
        config.appearance.icon_size = 64;

        let written = saved(
            WRITTEN_BY_HAND,
            &toml::to_string_pretty(&config).expect("serialises"),
        );

        assert!(written.contains("icon_size = 64"), "the change did not land");
        for kept in [
            "# My dock. Do not let the daemon eat this.",
            "# dark, because the desk is dark",
            "# The dock I use for work.",
            "# the one I live in",
        ] {
            assert!(written.contains(kept), "{kept} was eaten:\n{written}");
        }
        // The keys the struct defaults in are new to this file, so they are
        // appended — that is the file gaining them, not losing anything.
        assert!(written.contains("magnification"));
        assert!(
            toml::from_str::<crate::config::Config>(&written).is_ok(),
            "what was written back does not parse:\n{written}"
        );
    }
}
