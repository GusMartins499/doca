//! What the day accumulated, as opposed to what the user chose.
//!
//! The config file is choices: a goal of two litres, a theme, which docks
//! pin what. This is the other half — the millilitres already drunk, which
//! nobody chose and nothing should offer to edit. Keeping the two apart is
//! what lets the preferences window show the whole config without showing a
//! counter, and what lets the daemon come back up still knowing what the
//! afternoon had got to.
//!
//! It lives under `$XDG_STATE_HOME` for the reason that directory exists: it
//! is the state a program keeps between runs and the user is not expected to
//! read, as against `$XDG_CONFIG_HOME`, which is a file they may well edit by
//! hand. A daemon that wrote its counters into `config.toml` would rewrite
//! that file every time a glass was counted, with the user's own comments and
//! key order in it.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use serde::{Deserialize, Serialize};

/// Where the state file lives.
///
/// `$XDG_STATE_HOME`, then `~/.local/state` — which is what the base
/// directory specification says that variable defaults to.
pub fn state_path() -> PathBuf {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("doca/state.toml")
}

/// Everything the widgets want to still know tomorrow.
///
/// Every field is `#[serde(default)]` and the struct is `Default`, so a file
/// written by an older build — or no file at all — reads as "nothing counted
/// yet" rather than as a parse error.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct State {
    /// The day the counts below belong to, as days since the epoch where the
    /// user is. Zero for a state that has never been dated.
    #[serde(default)]
    pub day: i64,
    #[serde(default)]
    pub water: WaterState,
    /// The days that have ended, oldest first.
    ///
    /// **Not day-scoped**, which is why `forget_the_day` leaves it alone: it
    /// is the one thing a new day adds to rather than clears.
    #[serde(default)]
    pub closed: Vec<Closed>,
}

/// A day that ended, and what it came to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Closed {
    /// Days since the epoch where the user is, the same clock as [`State::day`].
    pub day: i64,
    pub ml: u32,
    /// What that day was being measured against, kept *with* it.
    ///
    /// A goal raised in March must not redraw February: a bar is a share of
    /// what was aimed at on the day, and asking the config what the goal is
    /// now would rewrite every day in the history whenever somebody changed
    /// their mind.
    ///
    /// Named for what it is rather than `goal`, and that is not decoration:
    /// `goal` is what the user *sets*, and a leaf of that name in this file
    /// would be a setting living where the accumulation lives — which
    /// `nothing_the_day_accumulated_is_also_something_the_user_sets` refuses,
    /// and was right to.
    pub aimed_at: u32,
}

/// How many ended days are kept.
///
/// The panel wants a week, so why a month: because the history is **sparse**.
/// A day the dock never ran leaves no entry at all, so a cap of seven entries
/// would answer "the last seven days it ran", which on a laptop that spent a
/// week shut is not a week. The cap has to be on entries while the question
/// is about calendar days, and a month of them makes a seven-day window
/// always answerable. Thirty-one rows of three numbers is under a kilobyte,
/// and bounded, which is the other half of the point.
pub const KEPT_DAYS: usize = 31;

/// How many days the week shown in the panel covers, today included.
pub const WEEK: usize = 7;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaterState {
    /// Millilitres drunk today.
    #[serde(default)]
    pub ml: u32,
    /// What today is being measured against, for the day to be closed with.
    ///
    /// Beside the count rather than only in the config, so that the day can be
    /// closed into the history without asking what the goal is *now*. Named
    /// `aimed_at` for the reason [`Closed::aimed_at`] gives.
    #[serde(default)]
    pub aimed_at: u32,
}

impl State {
    /// Forget everything that belonged to a day that has ended.
    ///
    /// **The one place the turn of the day is spelled out.** No widget asks
    /// what day it is or compares a stored date to today; the hub notices the
    /// turn once, this says what a new day drops, and every widget is simply
    /// handed the state that comes out. So yesterday's water cannot count
    /// towards today's goal, and the next counter added cannot forget to
    /// implement a rule it never had to know about.
    ///
    /// Anything that is *not* day-scoped — a total kept for ever, a streak —
    /// goes in a field this leaves alone, and the comment above it says so.
    fn forget_the_day(&mut self, ended: i64) {
        // The day goes into the history on its way out, which is where the
        // week in the panel comes from. A day that never had anything in it
        // is not a day worth a row.
        if ended > 0 && self.water.ml > 0 {
            self.closed.push(Closed {
                day: ended,
                ml: self.water.ml,
                aimed_at: self.water.aimed_at,
            });
            self.closed.sort_by_key(|closed| closed.day);
            self.closed.dedup_by_key(|closed| closed.day);
            let over = self.closed.len().saturating_sub(KEPT_DAYS);
            self.closed.drain(..over);
        }
        self.water = WaterState::default();
    }

    /// What was drunk on each of the last [`WEEK`] days, oldest first, today
    /// last.
    ///
    /// Days the dock never saw come back as zero rather than being left out:
    /// a week of bars has seven places whether or not the machine was on, and
    /// a gap drawn as a gap is the truth. Today is read from the running
    /// count, not from the history, because today has not ended.
    pub fn week_to(&self, today: i64) -> Vec<u32> {
        let first = today - (WEEK as i64 - 1);
        (first..=today)
            .map(|day| {
                if day == today {
                    return self.water.ml;
                }
                self.closed
                    .iter()
                    .find(|closed| closed.day == day)
                    .map(|closed| closed.ml)
                    .unwrap_or(0)
            })
            .collect()
    }

    /// Carry this state onto `today`, dropping whatever belonged to the day
    /// before. Answers whether anything was actually dropped.
    ///
    /// A state already dated today is left exactly as it is, which is the
    /// case on every start within the same day.
    pub fn roll_to(&mut self, today: i64) -> bool {
        if self.day == today {
            return false;
        }
        // The same state, dated today and otherwise untouched: what this
        // would be if a new day dropped nothing. Comparing against it is how
        // a first run — which has a stale date and nothing to lose — is told
        // from a real midnight.
        let mut as_if_nothing_was_dropped = self.clone();
        as_if_nothing_was_dropped.day = today;

        let ended = self.day;
        self.day = today;
        self.forget_the_day(ended);
        *self != as_if_nothing_was_dropped
    }
}

/// Today where the user is, as days since the epoch.
///
/// The offset is read once and kept: it comes from running `date`, and the
/// hub asks what day it is on every tick.
pub struct Day {
    offset: i64,
}

impl Day {
    pub fn here() -> Self {
        Self {
            offset: crate::widgets::clock::local_offset_seconds(),
        }
    }

    #[cfg(test)]
    pub fn at_offset(offset: i64) -> Self {
        Self { offset }
    }

    pub fn today(&self) -> i64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|since| since.as_secs() as i64)
            .unwrap_or(0);
        (now + self.offset).div_euclid(86_400)
    }
}

/// The state file, and the state it holds.
///
/// The hub owns one of these and is the only writer, which is what makes the
/// writes safe without a lock: a counter is written when a click changes it
/// and when the day turns, never on a tick.
pub struct Store {
    path: PathBuf,
    held: State,
}

impl Store {
    /// Read the file, carry it onto today, and say in the log what happened.
    ///
    /// Never fails. A file that is missing, empty or corrupt is a day that
    /// starts from zero — the alternative is a daemon that will not start
    /// because of a counter, which is a dock the user cannot use because
    /// something went wrong with a glass of water.
    pub fn open(today: i64) -> Self {
        Self::open_at(&state_path(), today)
    }

    pub fn open_at(path: &Path, today: i64) -> Self {
        let mut held = read(path);
        if held.roll_to(today) {
            tracing::info!("a new day: yesterday's counters were put away");
        }
        Self {
            path: path.to_path_buf(),
            held,
        }
    }

    pub fn state(&self) -> &State {
        &self.held
    }

    pub fn day(&self) -> i64 {
        self.held.day
    }

    /// The state as it now is, saved if it is not what is already on disk.
    ///
    /// Compared first because the hub offers one of these after every action,
    /// and most actions change nothing a counter cares about.
    pub fn put(&mut self, state: State) {
        if state == self.held {
            return;
        }
        self.held = state;
        if let Err(e) = self.save() {
            // Worth saying once per failed write and not worth stopping for:
            // the dock in front of the user is still right, it is only
            // tomorrow that will have forgotten.
            tracing::warn!("cannot keep today's counters: {e:#}");
        }
    }

    /// Put yesterday away, and answer with the state every widget should now
    /// be holding.
    pub fn roll_to(&mut self, today: i64) -> &State {
        let mut rolled = self.held.clone();
        if rolled.roll_to(today) {
            tracing::info!("a new day: yesterday's counters were put away");
        }
        self.put(rolled);
        // `put` ignores a state equal to the one held, so this is the held
        // one either way.
        &self.held
    }

    fn save(&self) -> Result<()> {
        crate::atomic::write(&self.path, &toml::to_string_pretty(&self.held)?)
    }
}

/// What is in the file, or a state that has counted nothing.
fn read(path: &Path) -> State {
    let Ok(body) = std::fs::read_to_string(path) else {
        // Not a warning. No file is what the first run looks like.
        tracing::debug!("no state at {}, starting the day from zero", path.display());
        return State::default();
    };
    if body.trim().is_empty() {
        tracing::warn!("the state at {} is empty, starting from zero", path.display());
        return State::default();
    }
    match toml::from_str::<State>(&body) {
        Ok(state) => state,
        Err(e) => {
            tracing::warn!(
                "ignoring unreadable state at {}, starting from zero: {e}",
                path.display()
            );
            State::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory of this test's own. The state path comes from the
    /// environment, which tests share, so every test here names its file.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("doca-state-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn file(&self) -> PathBuf {
            self.0.join("state.toml")
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const TODAY: i64 = 20_735;

    fn water(day: i64, ml: u32) -> State {
        State {
            day,
            water: WaterState { ml, aimed_at: 0 },
            closed: Vec::new(),
        }
    }

    /// A day that knows what it is aiming at, which is what makes it worth
    /// filing when it ends.
    fn aiming(day: i64, ml: u32, at: u32) -> State {
        State {
            day,
            water: WaterState { ml, aimed_at: at },
            closed: Vec::new(),
        }
    }

    /// The whole of item 4: what the afternoon counted is there after a
    /// restart.
    #[test]
    fn what_was_counted_today_is_still_counted_after_a_restart() {
        let scratch = Scratch::new("restart");

        let mut store = Store::open_at(&scratch.file(), TODAY);
        store.put(water(TODAY, 6));

        let again = Store::open_at(&scratch.file(), TODAY);

        assert_eq!(again.state().water.ml, 6);
    }

    /// And the other half: it is not still counted tomorrow.
    #[test]
    fn yesterdays_water_does_not_count_towards_todays_goal() {
        let scratch = Scratch::new("midnight");
        let mut store = Store::open_at(&scratch.file(), TODAY);
        store.put(water(TODAY, 6));

        let tomorrow = Store::open_at(&scratch.file(), TODAY + 1);

        assert_eq!(tomorrow.state().water.ml, 0);
        assert_eq!(tomorrow.day(), TODAY + 1);
    }

    /// The turn of the day while the daemon is up, rather than across a
    /// restart — a machine that has been awake since before midnight.
    #[test]
    fn the_day_turning_under_a_running_daemon_puts_yesterday_away() {
        let scratch = Scratch::new("awake");
        let mut store = Store::open_at(&scratch.file(), TODAY);
        store.put(aiming(TODAY, 1500, 2000));

        let rolled = store.roll_to(TODAY + 1).clone();

        assert_eq!(
            rolled.water,
            WaterState::default(),
            "today started from yesterday's count"
        );
        assert_eq!(
            rolled.closed,
            vec![Closed { day: TODAY, ml: 1500, aimed_at: 2000 }],
            "yesterday was dropped instead of being filed"
        );
        assert_eq!(
            Store::open_at(&scratch.file(), TODAY + 1).state().water.ml,
            0,
            "the turn of the day was never written down"
        );
    }

    #[test]
    fn a_day_that_has_not_turned_leaves_the_counters_alone() {
        let mut state = water(TODAY, 6);

        assert!(!state.roll_to(TODAY), "a day that did not turn was reported as turning");
        assert_eq!(state.water.ml, 6);
    }

    #[test]
    fn a_first_run_has_nothing_to_forget_and_says_so() {
        let mut fresh = State::default();

        assert!(
            !fresh.roll_to(TODAY),
            "a state that counted nothing reported losing something"
        );
        assert_eq!(fresh.day, TODAY, "the day was not written down");
    }

    /// A day with nothing in it is not a day worth a row — a week of bars
    /// should have gaps where there were gaps, not rows of zero padding the
    /// file out.
    #[test]
    fn a_day_that_counted_nothing_is_not_filed() {
        let mut state = aiming(TODAY, 0, 2000);

        state.roll_to(TODAY + 1);

        assert!(state.closed.is_empty());
    }

    /// The history is the one thing a new day adds to rather than clears,
    /// and `forget_the_day` has to go on leaving it alone as counters are
    /// added beside it.
    #[test]
    fn the_history_outlives_every_day_that_is_forgotten() {
        let mut state = aiming(TODAY, 500, 2000);

        for day in 1..=5i64 {
            state.roll_to(TODAY + day);
            state.water = WaterState { ml: 100 * day as u32, aimed_at: 2000 };
        }

        assert_eq!(state.closed.len(), 5);
        assert_eq!(state.closed.first().map(|first| first.day), Some(TODAY));
        assert_eq!(state.closed.last().map(|last| last.ml), Some(400));
    }

    /// The file cannot grow for ever, and what falls off the end is the
    /// oldest — a month of days, which is what makes a seven-day window
    /// answerable even on a machine that spent a fortnight shut.
    #[test]
    fn the_history_is_capped_and_it_is_the_oldest_that_goes() {
        let mut state = aiming(TODAY, 1, 2000);

        for day in 1..=(KEPT_DAYS as i64 + 10) {
            state.roll_to(TODAY + day);
            state.water = WaterState { ml: day as u32 + 1, aimed_at: 2000 };
        }

        assert_eq!(state.closed.len(), KEPT_DAYS);
        assert_eq!(
            state.closed.first().map(|first| first.day),
            Some(TODAY + 10),
            "the newest were dropped instead of the oldest"
        );
    }

    /// A machine that was shut for a week turns the day once, not seven
    /// times, so the days in between were never seen and are not invented.
    #[test]
    fn days_the_dock_never_saw_are_not_filed_as_zero() {
        let mut state = aiming(TODAY, 1200, 2000);

        state.roll_to(TODAY + 7);

        assert_eq!(state.closed.len(), 1, "{:?}", state.closed);
        assert_eq!(state.closed[0].day, TODAY);
    }

    /// And the other side of that: the week asks for seven *calendar* days,
    /// so the days with no row come back as zero rather than shifting the
    /// ones that have rows into the wrong place.
    #[test]
    fn the_week_has_seven_places_whatever_the_history_has() {
        let mut state = aiming(TODAY, 0, 2000);
        state.closed = vec![
            Closed { day: TODAY - 6, ml: 2000, aimed_at: 2000 },
            Closed { day: TODAY - 4, ml: 500, aimed_at: 2000 },
            // Older than the window, and so none of its business.
            Closed { day: TODAY - 20, ml: 1750, aimed_at: 2000 },
        ];
        state.water = WaterState { ml: 750, aimed_at: 2000 };

        let week = state.week_to(TODAY);

        assert_eq!(week.len(), WEEK);
        assert_eq!(week, vec![2000, 0, 500, 0, 0, 0, 750]);
    }

    /// Today is read from the running count and not from the history,
    /// because today has not ended — a press has to show in the last bar
    /// now, not at midnight.
    #[test]
    fn the_last_place_in_the_week_is_today_as_it_stands() {
        let mut state = aiming(TODAY, 250, 2000);
        state.closed = vec![Closed { day: TODAY, ml: 9999, aimed_at: 2000 }];

        assert_eq!(state.week_to(TODAY).last(), Some(&250));
    }

    #[test]
    fn a_state_file_nobody_has_written_yet_is_a_day_from_zero() {
        let scratch = Scratch::new("missing");

        let store = Store::open_at(&scratch.file(), TODAY);

        assert_eq!(store.state(), &water(TODAY, 0));
        assert!(!scratch.file().exists(), "reading wrote a file");
    }

    #[test]
    fn an_empty_state_file_does_not_bring_the_daemon_down() {
        let scratch = Scratch::new("empty");
        std::fs::write(scratch.file(), "").unwrap();

        assert_eq!(Store::open_at(&scratch.file(), TODAY).state().water.ml, 0);
    }

    #[test]
    fn a_half_written_state_file_does_not_bring_the_daemon_down() {
        let scratch = Scratch::new("torn");
        std::fs::write(scratch.file(), "day = 20735\n[water]\nglass").unwrap();

        assert_eq!(Store::open_at(&scratch.file(), TODAY).state().water.ml, 0);
    }

    /// A file from a build that knew fields this one does not.
    #[test]
    fn a_state_from_another_build_keeps_the_fields_this_one_knows() {
        let scratch = Scratch::new("newer");
        std::fs::write(
            scratch.file(),
            "day = 20735\nstreak = 9\n[water]\nml = 4\n[sleep]\nhours = 7\n",
        )
        .unwrap();

        assert_eq!(Store::open_at(&scratch.file(), TODAY).state().water.ml, 4);
    }

    #[test]
    fn a_state_that_changed_nothing_is_not_written_again() {
        let scratch = Scratch::new("quiet");
        let mut store = Store::open_at(&scratch.file(), TODAY);
        store.put(water(TODAY, 1));
        let written = std::fs::metadata(scratch.file()).unwrap().len();

        store.put(water(TODAY, 1));

        assert_eq!(std::fs::metadata(scratch.file()).unwrap().len(), written);
        assert_eq!(store.state().water.ml, 1);
    }

    #[test]
    fn a_state_that_cannot_be_written_does_not_bring_the_daemon_down() {
        let mut store = Store::open_at(Path::new("/doca-cannot-write-here/state.toml"), TODAY);

        store.put(water(TODAY, 3));

        assert_eq!(
            store.state().water.ml, 3,
            "the dock in front of the user should still be right"
        );
    }

    /// The line between the two files, as a thing a test can fail on.
    ///
    /// The preferences window draws a control for every pair in
    /// `doca_ipc::widget_key::ALL` and for nothing else, so a counter can
    /// only appear in that window by first becoming a *setting*. This is what
    /// would notice: the names the state file writes and the names the config
    /// file writes have to stay strangers.
    #[test]
    fn nothing_the_day_accumulated_is_also_something_the_user_sets() {
        let counted = leaves(&toml::to_string(&water(TODAY, 3)).unwrap());
        let chosen =
            leaves(&toml::to_string(&crate::config::WidgetSettings::default()).unwrap());

        assert!(counted.contains(&"ml".to_string()), "{counted:?}");
        assert!(chosen.contains(&"goal".to_string()), "{chosen:?}");
        for name in &counted {
            assert!(
                !chosen.contains(name),
                "{name} is both something the day counted and something the \
                 user sets, so it would show up as a control in the \
                 preferences window"
            );
        }
    }

    /// The `key = value` names in a TOML document, table headers aside — the
    /// two files share a `[water]` table and nothing inside it.
    fn leaves(document: &str) -> Vec<String> {
        document
            .lines()
            .filter_map(|line| line.split_once('='))
            .map(|(name, _)| name.trim().to_string())
            .collect()
    }

    #[test]
    fn the_state_file_sits_beside_the_other_state_and_not_beside_the_config() {
        let path = state_path();

        assert!(path.ends_with("doca/state.toml"), "{}", path.display());
        assert!(
            !path.to_string_lossy().contains(".config"),
            "the state landed in the config directory: {}",
            path.display()
        );
    }

    /// The day is the user's, not UTC's: at one in the morning in Sao Paulo
    /// it is still the day before in UTC, and the water goal has to belong to
    /// the day the user is having.
    #[test]
    fn the_day_is_counted_where_the_user_is() {
        let here = Day::at_offset(0);
        let three_hours_behind = Day::at_offset(-3 * 3600);

        assert_eq!(here.today(), here.today(), "the day moved between two reads");
        assert!(
            (here.today() - three_hours_behind.today()).abs() <= 1,
            "two offsets three hours apart landed more than a day apart"
        );
        assert!(here.today() > 20_000, "the epoch is not today");
    }
}
