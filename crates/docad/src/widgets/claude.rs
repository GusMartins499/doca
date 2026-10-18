//! Claude Code's usage, in one of three views (#32).
//!
//! One widget with a mode rather than three in the catalogue: the three read
//! the same files, and three ids would be three things to keep in step.
//!
//! # Where the limit comes from, decided
//!
//! It does not, and the tile does not pretend otherwise. Usage can be
//! counted from Claude Code's own files (`usage`); the plan's ceiling is not
//! in them, nor anywhere else on disk — measured, see `usage`. A percentage
//! needs a denominator, and an invented one is the thing the issue asked
//! this widget never to show. So:
//!
//! - With no budget set, every mode shows what was spent, as a number, and
//!   no bar.
//! - With `claude.budget` set — the user's own figure, in tokens — the mode
//!   shows its share of that budget, with the bar, and says it is *of the
//!   budget*. It is the user's number standing in for the plan's, by their
//!   choice, and the tile does not call it the plan's limit.
//!
//! # Near the limit
//!
//! At [`NEAR`] of the budget the tile marks itself, the way a water tile
//! marks a goal reached. Four fifths: early enough to change what you do in
//! the hour left, late enough not to be lit all afternoon.
//!
//! # Personal data
//!
//! Only counts are read, kept or shown. See `usage` for what is parsed and
//! dropped. Nothing is logged but that the files could not be understood,
//! once.

use std::sync::mpsc;
use std::time::Duration;

use chrono::{DateTime, Duration as Span, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use doca_ipc::{claude_mode, claude_window, Body, WidgetState, NO_PROGRESS};

use super::usage::{self, Found, Ledger, Spend, Tokens};
use super::Widget;
use crate::config::ClaudeSettings;

/// The share of the budget at which the tile marks itself.
pub const NEAR: f64 = 0.8;

/// How often the files are read again: a minute, as the primo.dock does.
/// Reading is from where the last read stopped, so this costs what was
/// written in that minute and not the history.
const REFRESH: Duration = Duration::from_secs(60);

/// What the reading thread hands over.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub found: Found,
    pub spends: Vec<Spend>,
}

pub struct Claude {
    settings: ClaudeSettings,
    zone: Tz,
    reading: Option<Reading>,
    readings: Option<mpsc::Receiver<Reading>>,
}

impl Claude {
    pub fn new(settings: ClaudeSettings) -> Self {
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("claude-usage".into())
            .spawn(move || read_forever(tx));
        if let Err(e) = spawned {
            tracing::warn!("cannot start reading Claude Code's usage: {e}");
        }
        Self {
            settings,
            zone: super::calendar::local_zone(),
            reading: None,
            readings: Some(rx),
        }
    }
}

impl Widget for Claude {
    fn id(&self) -> &str {
        "claude"
    }

    /// The reading is a minute old at worst; this only has to notice a
    /// session closing or a day turning.
    fn interval(&self) -> Duration {
        Duration::from_secs(15)
    }

    fn poll(&mut self) -> WidgetState {
        if let Some(readings) = &self.readings {
            loop {
                match readings.try_recv() {
                    Ok(reading) => self.reading = Some(reading),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        self.readings = None;
                        break;
                    }
                }
            }
        }
        state_at(self.reading.as_ref(), Utc::now(), self.zone, &self.settings)
    }

    fn adopt(&mut self, settings: &crate::config::WidgetSettings) {
        self.settings = settings.claude.clone();
    }
}

fn read_forever(tx: mpsc::Sender<Reading>) {
    let Some(root) = usage::claude_dir() else {
        let _ = tx.send(Reading { found: Found::NoFolder, spends: Vec::new() });
        return;
    };
    let mut ledger = Ledger::default();
    let mut said = false;
    loop {
        let found = ledger.read(&root, Utc::now());
        if found == Found::NotUnderstood && !said {
            tracing::warn!(
                "Claude Code's files under {} carry usage this build cannot read; \
                 the format may have changed",
                root.display()
            );
            said = true;
        }
        let reading = Reading { found, spends: ledger.spends().to_vec() };
        if tx.send(reading).is_err() {
            return;
        }
        std::thread::sleep(REFRESH);
    }
}

/// What the tile shows at `now`, and what the panel breaks it into.
pub fn state_at(reading: Option<&Reading>, now: DateTime<Utc>, zone: Tz, settings: &ClaudeSettings) -> WidgetState {
    let mode = claude_mode::resolve(&settings.mode);
    let body = |label: String, detail: String, tokens: Tokens, turns: String, budget: u32| {
        let (label, detail, progress, near) = if budget == 0 || label.is_empty() {
            (if label.is_empty() { "Claude".to_string() } else { label }, detail, NO_PROGRESS, false)
        } else {
            let share = tokens.spent() as f64 / budget as f64;
            (
                format!("{:.0}%", (share * 100.0).floor()),
                format!("{} of {} · {detail}", compact(tokens.spent()), compact(budget as u64)),
                share.clamp(0.0, 1.0),
                share >= NEAR,
            )
        };
        WidgetState::new(
            "claude",
            Body::Claude(doca_ipc::Claude {
                mode: mode.to_string(),
                label,
                detail,
                progress,
                near,
                input: tokens.input,
                output: tokens.output,
                cache_write: tokens.cache_write,
                cache_read: tokens.cache_read,
                turns,
            }),
        )
    };
    // Said in words, never as a zero: a zero would read as a quiet day. An
    // empty label is how `body` is told there is no number to show.
    let missing = |words: &str| body(String::new(), words.to_string(), Tokens::default(), String::new(), 0);
    let Some(reading) = reading else {
        return missing("");
    };
    match reading.found {
        Found::NoFolder => return missing("Claude Code not found"),
        Found::Empty => return missing("no usage yet"),
        Found::NotUnderstood => return missing("usage format not understood"),
        Found::Read => {}
    }

    let until_now = now + Span::seconds(1);
    let (tokens, what, turns) = match mode {
        claude_mode::FIVE_HOURS => match usage::session_open_at(&reading.spends, now) {
            Some(opened) => {
                let closes = (opened + Span::hours(5)).with_timezone(&zone).format("%H:%M").to_string();
                (
                    usage::between(&reading.spends, opened, until_now),
                    format!("session · until {closes}"),
                    closes,
                )
            }
            None => (Tokens::default(), "no session open".to_string(), String::new()),
        },
        claude_mode::WEEK => (
            usage::between(&reading.spends, now - Span::days(7), until_now),
            "last 7 days".to_string(),
            String::new(),
        ),
        _ => match claude_window::resolve(&settings.window) {
            claude_window::FIVE_HOURS => (
                usage::between(&reading.spends, now - Span::hours(5), until_now),
                "last 5 hours".to_string(),
                String::new(),
            ),
            claude_window::TODAY => (
                usage::between(&reading.spends, midnight_of(now, zone), until_now),
                "today".to_string(),
                "midnight".to_string(),
            ),
            _ => (
                usage::between(&reading.spends, now - Span::days(7), until_now),
                "last 7 days".to_string(),
                String::new(),
            ),
        },
    };
    body(compact(tokens.spent()), what, tokens, turns, settings.budget)
}

fn midnight_of(now: DateTime<Utc>, zone: Tz) -> DateTime<Utc> {
    let day = now.with_timezone(&zone).date_naive();
    zone.from_local_datetime(&day.and_time(NaiveTime::MIN))
        .earliest()
        .map(|at| at.with_timezone(&Utc))
        .unwrap_or(now)
}

/// Tokens as a tile can show them — the one spelling the bar shares.
pub fn compact(tokens: u64) -> String {
    doca_ipc::compact_tokens(tokens)
}

#[cfg(test)]
mod tests {
    use super::super::Drawn;
    use super::*;

    const SAO_PAULO: Tz = chrono_tz::America::Sao_Paulo;

    fn at(d: u32, h: u32, m: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, d, h, m, 0).unwrap()
    }

    fn spend(when: DateTime<Utc>, input: u64) -> Spend {
        Spend { at: when, tokens: Tokens { input, cache_read: 1_000_000, ..Tokens::default() } }
    }

    fn read(spends: Vec<Spend>) -> Reading {
        Reading { found: Found::Read, spends }
    }

    fn settings(mode: &str, window: &str, budget: u32) -> ClaudeSettings {
        ClaudeSettings { mode: mode.to_string(), window: window.to_string(), budget }
    }

    // 21:00 UTC is 18:00 in São Paulo.
    fn now() -> DateTime<Utc> {
        at(17, 21, 0)
    }

    fn day() -> Reading {
        read(vec![
            spend(at(9, 12, 0), 5_000_000),
            spend(at(17, 4, 0), 300_000),
            spend(at(17, 18, 30), 1_200_000),
            spend(at(17, 20, 0), 1_200_000),
        ])
    }

    #[test]
    fn the_session_mode_counts_the_open_session_and_says_when_it_closes() {
        let state = state_at(Some(&day()), now(), SAO_PAULO, &settings("five-hours", "5h", 0));
        assert_eq!(state.label(), "2.4M");
        assert_eq!(state.detail(), "session · until 20:30", "18:30 UTC opened it; 23:30 UTC is 20:30 there");
        assert_eq!(state.progress(), NO_PROGRESS, "no budget, no bar");
    }

    #[test]
    fn with_no_session_open_it_says_so_rather_than_counting_an_old_one() {
        let state = state_at(Some(&day()), at(18, 2, 0), SAO_PAULO, &settings("five-hours", "5h", 0));
        assert_eq!((state.label().as_str(), state.detail().as_str()), ("0", "no session open"));
    }

    #[test]
    fn the_week_and_the_token_windows_count_what_they_say() {
        let week = state_at(Some(&day()), now(), SAO_PAULO, &settings("week", "5h", 0));
        assert_eq!((week.label().as_str(), week.detail().as_str()), ("2.7M", "last 7 days"));

        let today = state_at(Some(&day()), now(), SAO_PAULO, &settings("tokens", "today", 0));
        // 04:00 UTC on the 17th is the 17th's 01:00 in São Paulo: today.
        assert_eq!((today.label().as_str(), today.detail().as_str()), ("2.7M", "today"));

        let five = state_at(Some(&day()), now(), SAO_PAULO, &settings("tokens", "5h", 0));
        assert_eq!(five.label(), "2.4M");
    }

    #[test]
    fn a_budget_is_a_share_with_a_bar_and_lights_up_near_its_end() {
        let state = state_at(Some(&day()), now(), SAO_PAULO, &settings("five-hours", "5h", 3_000_000));
        assert_eq!(state.label(), "80%");
        assert_eq!(state.detail(), "2.4M of 3M · session · until 20:30");
        assert!((state.progress() - 0.8).abs() < 1e-9);
        assert!(state.active(), "at four fifths the tile marks itself");

        let calm = state_at(Some(&day()), now(), SAO_PAULO, &settings("five-hours", "5h", 10_000_000));
        assert!(!calm.active());

        let over = state_at(Some(&day()), now(), SAO_PAULO, &settings("week", "5h", 1_000_000));
        assert_eq!(over.label(), "270%", "over budget says how far over");
        assert_eq!(over.progress(), 1.0, "and the bar stops at full");
    }

    #[test]
    fn missing_and_misread_are_said_in_words_never_as_zero() {
        let s = settings("week", "5h", 0);
        let said = |found| state_at(Some(&Reading { found, spends: Vec::new() }), now(), SAO_PAULO, &s).detail();
        assert_eq!(said(Found::NoFolder), "Claude Code not found");
        assert_eq!(said(Found::Empty), "no usage yet");
        assert_eq!(said(Found::NotUnderstood), "usage format not understood");
    }

    #[test]
    fn the_panel_gets_the_breakdown_and_when_the_window_turns() {
        let state = state_at(Some(&day()), now(), SAO_PAULO, &settings("five-hours", "5h", 0));
        let Ok(Body::Claude(claude)) = state.body() else { panic!("not a usage body") };
        assert_eq!(claude.mode, "five-hours");
        assert_eq!((claude.input, claude.cache_read), (2_400_000, 2_000_000), "two replies in the session");
        assert_eq!(claude.turns, "20:30");

        let today = state_at(Some(&day()), now(), SAO_PAULO, &settings("tokens", "today", 0));
        let Ok(Body::Claude(claude)) = today.body() else { panic!("not a usage body") };
        assert_eq!(claude.turns, "midnight");
        let week = state_at(Some(&day()), now(), SAO_PAULO, &settings("week", "today", 0));
        let Ok(Body::Claude(claude)) = week.body() else { panic!("not a usage body") };
        assert_eq!(claude.turns, "", "seven days roll; they do not turn");
    }

    #[test]
    fn tokens_read_at_a_glance() {
        for (tokens, shown) in [
            (950, "950"),
            (1_000, "1k"),
            (12_345, "12k"),
            (2_412_887, "2.4M"),
            (24_128_870, "24M"),
            (3_000_000, "3M"),
            (1_500_000_000, "1.5B"),
        ] {
            assert_eq!(compact(tokens), shown);
        }
    }
}
