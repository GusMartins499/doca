//! The plan's own limits, as Claude Code hands them to its status line (#76).
//!
//! # Where the numbers come from
//!
//! Since 2.1.x Claude Code runs the command named in `statusLine.command` of
//! its `settings.json` and writes a JSON object to its stdin, which since
//! then carries `rate_limits.five_hour` and `rate_limits.seven_day`, each
//! with `used_percentage` (0–100) and `resets_at` (Unix seconds). Only on
//! Pro and Max plans, only after the session's first reply, and either
//! window can be missing on its own. It is never written to disk: the status
//! line command is the only thing that sees it, and only while a session is
//! open.
//!
//! So `docad claude-statusline` is that command. It keeps the four numbers in
//! [`limits_path`] and nothing else — the rest of the object is the session's
//! working directory, its transcript's path, its model, and is parsed past
//! and dropped. No network, no API, no token: the rule #32 set.
//!
//! # Not stepping on a status line that was there
//!
//! A user who already had a `statusLine` keeps it. Installing records it in
//! [`saved_path`] before anything in `settings.json` changes, and the command
//! here runs it with the same stdin and lets its output be the one shown.
//! Uninstalling puts it back exactly, or removes the key if there was none.
//!
//! Installing is only ever done because `claude.plan` was switched on, and
//! with it off nothing under `~/.claude` is so much as opened unless this
//! module installed something there before.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The first argument that makes `docad` the status line instead of the
/// daemon.
pub const MODE: &str = "claude-statusline";

/// One window's share of the plan, as Claude Code reports it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Window {
    /// 0 to 100.
    pub used_percentage: f64,
    /// Unix seconds.
    pub resets_at: i64,
}

/// The two windows, each of which may be missing on its own.
///
/// This is the whole of what is written to [`limits_path`]: four numbers.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Limits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub five_hour: Option<Window>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seven_day: Option<Window>,
}

impl Limits {
    /// What was last recorded, or nothing for a file that is missing or
    /// does not parse — either way the tile falls back to the budget.
    pub fn read(path: &Path) -> Option<Self> {
        serde_json::from_slice(&std::fs::read(path).ok()?).ok()
    }

    /// The windows in what Claude Code wrote to the status line's stdin.
    ///
    /// Read window by window, so one that is missing or malformed costs only
    /// itself. Nothing else in `input` is looked at.
    pub fn from_status_line(input: &[u8]) -> Self {
        let Ok(input) = serde_json::from_slice::<Value>(input) else {
            return Self::default();
        };
        let window = |name: &str| -> Option<Window> {
            let window = input.get("rate_limits")?.get(name)?;
            let used = window.get("used_percentage")?.as_f64()?;
            let resets_at = window.get("resets_at")?.as_f64()?;
            (used.is_finite() && used >= 0.0 && resets_at > 0.0).then(|| Window {
                used_percentage: used.min(100.0),
                resets_at: resets_at as i64,
            })
        };
        Self { five_hour: window("five_hour"), seven_day: window("seven_day") }
    }

    /// These windows over the ones recorded before.
    ///
    /// The limits are the account's, not the session's: a session opened a
    /// minute ago has no `rate_limits` until its first reply, and must not
    /// wipe out what another session reported. A window kept this way still
    /// expires by its own `resets_at`.
    fn over(self, before: Self) -> Self {
        Self {
            five_hour: self.five_hour.or(before.five_hour),
            seven_day: self.seven_day.or(before.seven_day),
        }
    }
}

/// `$XDG_RUNTIME_DIR/doca/claude-limits.json`.
///
/// The runtime directory because these numbers are only true while a
/// session reports them: they should not outlive a logout, and it is a
/// tmpfs, so a write on every status line refresh costs no disk. Without the
/// variable there is nowhere to put them, and the tile keeps the budget.
pub fn limits_path() -> Option<PathBuf> {
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|dir| !dir.is_empty())
        .map(|dir| PathBuf::from(dir).join("doca/claude-limits.json"))
}

/// Where the status line that was there before is kept, beside the state.
///
/// Not the runtime directory: it has to survive a reboot, or switching the
/// option off after one would have nothing to give back.
pub fn saved_path() -> PathBuf {
    crate::state::state_path().with_file_name("claude-statusline.json")
}

/// Claude Code's `settings.json`, under `$CLAUDE_CONFIG_DIR` when set.
pub fn settings_path() -> Option<PathBuf> {
    crate::widgets::usage::claude_dir().map(|dir| dir.join("settings.json"))
}

/// Record what Claude Code sent, merged over what was recorded before.
///
/// Input with no window in it writes nothing, and neither does input that
/// changes nothing — the status line refreshes several times a second while
/// Claude Code is busy.
pub fn record(input: &[u8], path: &Path) -> Result<()> {
    let now = Limits::from_status_line(input);
    if now == Limits::default() {
        return Ok(());
    }
    let before = Limits::read(path).unwrap_or_default();
    let merged = now.over(before);
    if Limits::read(path) == Some(merged) {
        return Ok(());
    }
    crate::atomic::write(path, serde_json::to_vec(&merged)?)
}

/// `docad claude-statusline`: what Claude Code runs.
pub fn run_here() -> i32 {
    let mut input = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut input);
    let stdout = std::io::stdout();
    run(&input, limits_path().as_deref(), &saved_path(), &mut stdout.lock())
}

/// Record the limits, then hand the same input to the status line that was
/// there before, if any, and pass on what it prints and how it exits.
///
/// Recording never gets in the way: a runtime directory that cannot be
/// written is a tile that keeps the budget, not a status line that broke.
pub fn run(input: &[u8], limits: Option<&Path>, saved: &Path, out: &mut impl Write) -> i32 {
    if let Some(limits) = limits {
        let _ = record(input, limits);
    }
    let Some(command) = previous(saved).as_ref().and_then(command_of) else {
        return 0;
    };
    // A line that is this one again — saved by a second install whose first
    // was lost — would call itself until the machine ran out of processes.
    if is_ours_command(&command) {
        return 0;
    }
    chain(&command, input, out).unwrap_or(1)
}

fn chain(command: &str, input: &[u8], out: &mut impl Write) -> Result<i32> {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(command)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("cannot run {command}"))?;
    // Written from a thread of its own: a command that prints before it has
    // read everything would otherwise fill one pipe while this waits on the
    // other.
    let mut stdin = child.stdin.take().context("no stdin")?;
    let input = input.to_vec();
    let feeding = std::thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    let output = child.wait_with_output()?;
    let _ = feeding.join();
    out.write_all(&output.stdout)?;
    out.flush()?;
    Ok(output.status.code().unwrap_or(1))
}

/// What `settings.json` had under `statusLine` before this module put its
/// own there, as it was written then.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Saved {
    /// `None` for a file that had no status line at all.
    previous: Option<Value>,
}

fn previous(saved: &Path) -> Option<Value> {
    let saved: Saved = serde_json::from_slice(&std::fs::read(saved).ok()?).ok()?;
    saved.previous
}

fn command_of(line: &Value) -> Option<String> {
    let command = line.get("command")?.as_str()?.trim();
    (!command.is_empty()).then(|| command.to_string())
}

fn is_ours_command(command: &str) -> bool {
    command.trim_end().ends_with(&format!(" {MODE}"))
}

fn is_ours(line: &Value) -> bool {
    command_of(line).is_some_and(|command| is_ours_command(&command))
}

/// The command line for this very binary, quoted for the shell Claude Code
/// runs it in.
pub fn command_for(exe: &Path) -> String {
    let exe = exe.to_string_lossy();
    format!("'{}' {MODE}", exe.replace('\'', r"'\''"))
}

/// Put `settings.json` where `claude.plan` says it should be.
///
/// Called when the daemon starts and when that one setting changes. A
/// failure is logged and is all it costs: the tile keeps the budget.
pub fn follow(on: bool) {
    let Some(settings) = settings_path() else {
        return;
    };
    let result = if on {
        match std::env::current_exe() {
            Ok(exe) => install(&settings, &saved_path(), &command_for(&exe)),
            Err(e) => Err(anyhow::Error::new(e).context("cannot tell where docad is")),
        }
    } else {
        uninstall(&settings, &saved_path())
    };
    if let Err(e) = result {
        tracing::warn!("Claude Code's status line was left as it was: {e:#}");
    }
}

/// Make `command` the status line, keeping the one that was there.
///
/// The old line is recorded before `settings.json` is touched, so a crash
/// between the two writes leaves it saved rather than lost. An install that
/// finds its own line there already only corrects the path to the binary,
/// which moves when it is rebuilt somewhere else.
pub fn install(settings: &Path, saved: &Path, command: &str) -> Result<()> {
    let mut document = read_settings(settings)?;
    let current = document.get("statusLine").cloned();

    if let Some(line) = current.as_ref().filter(|line| is_ours(line)) {
        if command_of(line).as_deref() == Some(command) {
            return Ok(());
        }
        document["statusLine"]["command"] = Value::from(command);
        return write_settings(settings, &document);
    }

    crate::atomic::write(saved, serde_json::to_vec_pretty(&Saved { previous: current.clone() })?)?;
    // Whatever else the old line was set up with — its padding — stays.
    let mut ours = match current {
        Some(Value::Object(line)) => line,
        _ => Map::new(),
    };
    ours.insert("type".to_string(), Value::from("command"));
    ours.insert("command".to_string(), Value::from(command));
    document.insert("statusLine".to_string(), Value::Object(ours));
    write_settings(settings, &document)
}

/// Give back the status line [`install`] replaced.
///
/// Nothing saved means nothing was installed, and then `settings.json` is
/// not opened. A line that is no longer this one — the user changed it since
/// — is theirs now and is left alone.
pub fn uninstall(settings: &Path, saved: &Path) -> Result<()> {
    if !saved.exists() {
        return Ok(());
    }
    let recorded: Saved = serde_json::from_slice(&std::fs::read(saved)?)
        .with_context(|| format!("{} does not say what to put back", saved.display()))?;
    let mut document = read_settings(settings)?;
    if document.get("statusLine").is_some_and(is_ours) {
        match recorded.previous {
            Some(line) => document.insert("statusLine".to_string(), line),
            None => document.remove("statusLine"),
        };
        write_settings(settings, &document)?;
    }
    std::fs::remove_file(saved).with_context(|| format!("cannot remove {}", saved.display()))
}

/// The settings as an object, empty for a file that is not there yet.
///
/// A file that is there and does not parse is refused: it is the user's,
/// and writing over what this could not read would lose it.
fn read_settings(path: &Path) -> Result<Map<String, Value>> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Map::new()),
        Err(e) => return Err(anyhow::Error::new(e).context(format!("cannot read {}", path.display()))),
    };
    match serde_json::from_str(&text) {
        Ok(Value::Object(document)) => Ok(document),
        Ok(_) => anyhow::bail!("{} is not a JSON object", path.display()),
        Err(e) => Err(anyhow::Error::new(e).context(format!("{} does not parse", path.display()))),
    }
}

/// Write the settings whole, in the order they were read.
///
/// Through a symlink rather than over it — a `settings.json` kept with the
/// rest of someone's dotfiles stays a link — and with the permissions the
/// file had.
fn write_settings(path: &Path, document: &Map<String, Value>) -> Result<()> {
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let permissions = std::fs::metadata(&target).ok().map(|meta| meta.permissions());
    let mut text = serde_json::to_string_pretty(document)?;
    text.push('\n');
    crate::atomic::write(&target, text)?;
    if let Some(permissions) = permissions {
        let _ = std::fs::set_permissions(&target, permissions);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory of this test's own, cleaned up after it. Every path in
    /// these tests is under one: nothing here goes near a real `~/.claude`.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("doca-status-line-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn at(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const BOTH: &str = r#"{
        "session_id": "abc", "cwd": "/home/someone/secret-project",
        "model": {"id": "claude-opus", "display_name": "Opus"},
        "transcript_path": "/home/someone/.claude/projects/x.jsonl",
        "rate_limits": {
            "five_hour": {"used_percentage": 72, "resets_at": 1762650000},
            "seven_day": {"used_percentage": 41.5, "resets_at": 1763000000}
        }
    }"#;

    const OURS: &str = "'/usr/bin/docad' claude-statusline";

    #[test]
    fn only_the_four_numbers_are_written_and_nothing_of_the_session() {
        let scratch = Scratch::new("four");
        let path = scratch.at("doca/claude-limits.json");
        record(BOTH.as_bytes(), &path).unwrap();

        let written: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(
            written,
            serde_json::json!({
                "five_hour": {"used_percentage": 72.0, "resets_at": 1762650000},
                "seven_day": {"used_percentage": 41.5, "resets_at": 1763000000}
            })
        );
        let text = std::fs::read_to_string(&path).unwrap();
        for leak in ["secret-project", "Opus", "abc", "jsonl"] {
            assert!(!text.contains(leak), "{leak} was written: {text}");
        }
    }

    #[test]
    fn input_without_rate_limits_writes_nothing_and_keeps_what_was_there() {
        let scratch = Scratch::new("none");
        let path = scratch.at("claude-limits.json");
        record(br#"{"cwd": "/somewhere"}"#, &path).unwrap();
        assert!(!path.exists(), "a file with no number in it was written");

        record(BOTH.as_bytes(), &path).unwrap();
        record(br#"{"cwd": "/somewhere", "rate_limits": {}}"#, &path).unwrap();
        record(b"not json at all", &path).unwrap();
        let kept = Limits::read(&path).unwrap();
        assert_eq!(kept.five_hour.map(|w| w.used_percentage), Some(72.0));
        assert_eq!(kept.seven_day.map(|w| w.used_percentage), Some(41.5));
    }

    #[test]
    fn a_single_window_replaces_its_own_and_leaves_the_other() {
        let scratch = Scratch::new("one");
        let path = scratch.at("claude-limits.json");
        record(BOTH.as_bytes(), &path).unwrap();
        record(
            br#"{"rate_limits": {"five_hour": {"used_percentage": 80, "resets_at": 1762660000}}}"#,
            &path,
        )
        .unwrap();

        let limits = Limits::read(&path).unwrap();
        assert_eq!(limits.five_hour, Some(Window { used_percentage: 80.0, resets_at: 1762660000 }));
        assert_eq!(limits.seven_day, Some(Window { used_percentage: 41.5, resets_at: 1763000000 }));

        let alone = Limits::from_status_line(
            br#"{"rate_limits": {"seven_day": {"used_percentage": 10, "resets_at": 1763000000}}}"#,
        );
        assert_eq!(alone.five_hour, None, "a missing window is not invented");
    }

    #[test]
    fn a_malformed_window_costs_only_itself() {
        let limits = Limits::from_status_line(
            br#"{"rate_limits": {
                "five_hour": {"used_percentage": "lots", "resets_at": 1762650000},
                "seven_day": {"used_percentage": 140, "resets_at": 1763000000}
            }}"#,
        );
        assert_eq!(limits.five_hour, None);
        assert_eq!(limits.seven_day.map(|w| w.used_percentage), Some(100.0), "over a hundred is a full plan");
    }

    #[test]
    fn the_status_line_that_was_there_gets_the_same_input_and_its_output_is_shown() {
        let scratch = Scratch::new("chain");
        let saved = scratch.at("claude-statusline.json");
        let limits = scratch.at("claude-limits.json");
        // Prints the model it was handed and exits as a status line that
        // failed would, so both are seen to come through.
        let previous = serde_json::json!({
            "type": "command",
            "command": "grep -o '\"display_name\": \"[A-Za-z]*\"'; exit 3",
        });
        std::fs::write(&saved, serde_json::to_vec(&Saved { previous: Some(previous) }).unwrap()).unwrap();

        let mut out = Vec::new();
        let code = run(BOTH.as_bytes(), Some(&limits), &saved, &mut out);

        assert_eq!(String::from_utf8(out).unwrap(), "\"display_name\": \"Opus\"\n");
        assert_eq!(code, 3);
        assert!(Limits::read(&limits).is_some(), "and the numbers were still recorded");
    }

    #[test]
    fn with_nothing_before_it_the_status_line_prints_nothing() {
        let scratch = Scratch::new("alone");
        let mut out = Vec::new();
        let code = run(BOTH.as_bytes(), None, &scratch.at("nothing-saved.json"), &mut out);
        assert_eq!((code, out.len()), (0, 0));
    }

    #[test]
    fn a_saved_line_that_is_this_one_is_never_called() {
        let scratch = Scratch::new("loop");
        let saved = scratch.at("claude-statusline.json");
        let previous = serde_json::json!({"type": "command", "command": OURS});
        std::fs::write(&saved, serde_json::to_vec(&Saved { previous: Some(previous) }).unwrap()).unwrap();
        let mut out = Vec::new();
        assert_eq!(run(BOTH.as_bytes(), None, &saved, &mut out), 0);
    }

    #[test]
    fn installing_over_a_status_line_keeps_it_and_switching_off_gives_it_back() {
        let scratch = Scratch::new("over");
        let settings = scratch.at("settings.json");
        let saved = scratch.at("state/claude-statusline.json");
        let before = "{\n  \"theme\": \"dark\",\n  \"statusLine\": {\n    \"type\": \"command\",\n    \
                      \"command\": \"~/bin/my-line.sh\",\n    \"padding\": 2\n  },\n  \"alwaysThinkingEnabled\": true\n}\n";
        std::fs::write(&settings, before).unwrap();

        install(&settings, &saved, OURS).unwrap();
        let during: Value = serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(
            during["statusLine"],
            serde_json::json!({"type": "command", "command": OURS, "padding": 2})
        );
        assert_eq!(during["theme"], "dark", "the rest of the file is the user's and stays");
        assert_eq!(command_of(&previous(&saved).unwrap()).as_deref(), Some("~/bin/my-line.sh"));

        uninstall(&settings, &saved).unwrap();
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), before, "put back as it was, in its order");
        assert!(!saved.exists());
    }

    #[test]
    fn installing_where_there_was_no_status_line_takes_it_away_again_after() {
        let scratch = Scratch::new("fresh");
        let settings = scratch.at("settings.json");
        let saved = scratch.at("claude-statusline.json");
        std::fs::write(&settings, "{\n  \"theme\": \"dark\"\n}\n").unwrap();

        install(&settings, &saved, OURS).unwrap();
        assert!(std::fs::read_to_string(&settings).unwrap().contains("claude-statusline"));
        uninstall(&settings, &saved).unwrap();
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), "{\n  \"theme\": \"dark\"\n}\n");

        let missing = scratch.at("none/settings.json");
        install(&missing, &saved, OURS).unwrap();
        uninstall(&missing, &saved).unwrap();
        assert_eq!(std::fs::read_to_string(&missing).unwrap(), "{}\n");
    }

    #[test]
    fn installing_twice_moves_the_path_and_keeps_what_was_saved_the_first_time() {
        let scratch = Scratch::new("twice");
        let settings = scratch.at("settings.json");
        let saved = scratch.at("claude-statusline.json");
        std::fs::write(&settings, r#"{"statusLine": {"type": "command", "command": "echo mine"}}"#).unwrap();

        install(&settings, &saved, OURS).unwrap();
        install(&settings, &saved, "'/opt/doca/docad' claude-statusline").unwrap();

        let during: Value = serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(during["statusLine"]["command"], "'/opt/doca/docad' claude-statusline");
        assert_eq!(command_of(&previous(&saved).unwrap()).as_deref(), Some("echo mine"));
    }

    #[test]
    fn a_line_the_user_changed_since_is_left_to_them() {
        let scratch = Scratch::new("theirs");
        let settings = scratch.at("settings.json");
        let saved = scratch.at("claude-statusline.json");
        std::fs::write(&settings, r#"{"statusLine": {"type": "command", "command": "echo old"}}"#).unwrap();
        install(&settings, &saved, OURS).unwrap();
        let theirs = r#"{"statusLine": {"type": "command", "command": "echo new"}}"#;
        std::fs::write(&settings, theirs).unwrap();

        uninstall(&settings, &saved).unwrap();
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), theirs);
        assert!(!saved.exists());
    }

    #[test]
    fn switched_off_with_nothing_installed_the_settings_are_not_opened() {
        let scratch = Scratch::new("never");
        // A directory where the file would be: opening it would fail loudly.
        let settings = scratch.at("settings.json");
        std::fs::create_dir_all(&settings).unwrap();
        uninstall(&settings, &scratch.at("claude-statusline.json")).unwrap();
    }

    #[test]
    fn a_settings_file_that_does_not_parse_is_never_written_over() {
        let scratch = Scratch::new("broken");
        let settings = scratch.at("settings.json");
        let saved = scratch.at("claude-statusline.json");
        std::fs::write(&settings, "{ \"theme\": \"dark\", // a comment\n}").unwrap();

        assert!(install(&settings, &saved, OURS).is_err());
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), "{ \"theme\": \"dark\", // a comment\n}");
        assert!(!saved.exists(), "nothing recorded for an install that did not happen");
    }

    #[test]
    fn a_settings_file_kept_as_a_link_stays_a_link() {
        let scratch = Scratch::new("link");
        let real = scratch.at("dotfiles-settings.json");
        let settings = scratch.at("settings.json");
        std::fs::write(&real, "{}").unwrap();
        std::os::unix::fs::symlink(&real, &settings).unwrap();

        install(&settings, &scratch.at("claude-statusline.json"), OURS).unwrap();
        assert!(std::fs::symlink_metadata(&settings).unwrap().file_type().is_symlink());
        assert!(std::fs::read_to_string(&real).unwrap().contains("claude-statusline"));
    }

    #[test]
    fn the_command_is_quoted_for_the_shell_and_recognised_as_this_one() {
        let command = command_for(Path::new("/home/o'brien/bin/docad"));
        assert_eq!(command, r"'/home/o'\''brien/bin/docad' claude-statusline");
        assert!(is_ours_command(&command));
        assert!(!is_ours_command("~/bin/my-line.sh"));
    }
}
