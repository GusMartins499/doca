//! The day, and what is next on the calendar.
//!
//! # Where the events come from, decided by measuring (#31)
//!
//! **Evolution Data Server, over D-Bus, with the zbus this daemon already
//! speaks.** The issue weighed three sources; on the machine this was written
//! on the measurements were:
//!
//! - EDS was already installed and running (`evolution-data-server` 3.44,
//!   which GNOME Shell itself depends on), with a Google account connected
//!   through GNOME Online Accounts and its three calendars already synced.
//!   Read from here, that is the user's real agenda with **no credential in
//!   Doca and no sync to write**.
//! - The cost the issue feared — linking `libecal` — does not apply: EDS is
//!   a D-Bus service, `OpenCalendar` and `GetObjectList` are two method
//!   calls, and they hand back iCalendar text. No new system library.
//! - That text still has to be read, and recurrence expanded, by us: EDS
//!   expands in its client library, not on the bus. That is `agenda`, and
//!   the expansion itself is the `rrule` crate's.
//!
//! Discarded: **`.ics` files** would have cost the same parser and asked the
//! user to keep files in sync by hand; **CalDAV directly** would have put a
//! credential, a network client and a sync engine in the daemon to read what
//! EDS already holds. Either is still possible later behind the same
//! `agenda`, as a source for a desktop without EDS.
//!
//! # Read only
//!
//! The only methods called are `Open`, `GetObjectList` and the source
//! manager's object listing. Doca writes to nobody's calendar.
//!
//! # Personal data
//!
//! Event titles are read to be shown on the tile and go nowhere else: never
//! to a log, never to disk. What is logged is that a calendar could not be
//! read, by its source id, once.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc;
use std::time::Duration;

use chrono::{DateTime, Datelike, Duration as Span, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use doca_ipc::{Body, Upcoming, WidgetState};

use super::agenda::{self, Occurrence};
use crate::config::CalendarSettings;
use super::Widget;

const SOURCES: &str = "org.gnome.evolution.dataserver.Sources5";
const SOURCE_MANAGER: &str = "/org/gnome/evolution/dataserver/SourceManager";
const SOURCE_INTERFACE: &str = "org.gnome.evolution.dataserver.Source";
const CALENDARS: &str = "org.gnome.evolution.dataserver.Calendar8";
const CALENDAR_FACTORY: &str = "/org/gnome/evolution/dataserver/CalendarFactory";
const FACTORY_INTERFACE: &str = "org.gnome.evolution.dataserver.CalendarFactory";
const CALENDAR_INTERFACE: &str = "org.gnome.evolution.dataserver.Calendar";

/// How far ahead is read, whatever is shown: the furthest any setting
/// looks. Changing the setting then takes effect on the next poll, from
/// what is already read, instead of waiting for EDS.
const AHEAD: i64 = doca_ipc::calendar_ahead::FURTHEST;

/// How often the calendars are read again.
///
/// The issue's bounds: not every second, not every hour — an event that
/// starts in two minutes has to be there. A minute is the most it can be
/// late by, and reading three calendars from EDS's own cache is cheap.
const REFRESH: Duration = Duration::from_secs(60);

/// What the reading thread has to say.
#[derive(Debug, Clone, PartialEq)]
pub enum Reading {
    /// Not read yet.
    Waiting,
    /// Nothing on this bus answers for calendars.
    NoService,
    /// The service is there and holds no calendar that is switched on.
    NoCalendars,
    /// The objects in the window, as iCalendar text.
    Read(Vec<String>),
}

pub struct Calendar {
    settings: CalendarSettings,
    zone: Tz,
    reading: Reading,
    readings: Option<mpsc::Receiver<Reading>>,
}

impl Calendar {
    pub fn new(settings: CalendarSettings) -> Self {
        let zone = local_zone();
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("calendar".into())
            .spawn(move || read_forever(zone, tx));
        if let Err(e) = spawned {
            tracing::warn!("cannot start reading the calendar: {e}");
        }
        Self { settings, zone, reading: Reading::Waiting, readings: Some(rx) }
    }

    fn take_readings(&mut self) {
        let Some(readings) = &self.readings else { return };
        loop {
            match readings.try_recv() {
                Ok(reading) => self.reading = reading,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.readings = None;
                    break;
                }
            }
        }
    }
}

impl Widget for Calendar {
    fn id(&self) -> &str {
        "calendar"
    }

    /// Often enough that the date turns within seconds of midnight and an
    /// event becomes "now" when it starts, from what was already read.
    fn interval(&self) -> Duration {
        Duration::from_secs(15)
    }

    fn poll(&mut self) -> WidgetState {
        self.take_readings();
        state_at(&self.reading, Utc::now(), self.zone, &self.settings)
    }

    fn adopt(&mut self, settings: &crate::config::WidgetSettings) {
        self.settings = settings.calendar.clone();
    }
}

/// How many of what is coming up are sent. The tile draws three at most and
/// the panel will want a few more; more than this is a list nobody reads on
/// a dock.
const UPCOMING: usize = 8;

/// What the tile shows at `now`, given what was read.
pub fn state_at(reading: &Reading, now: DateTime<Utc>, zone: Tz, settings: &CalendarSettings) -> WidgetState {
    let today = now.with_timezone(&zone);
    let (upcoming, missing) = match reading {
        Reading::Waiting => (Vec::new(), String::new()),
        Reading::NoService => (Vec::new(), "no calendar service".to_string()),
        Reading::NoCalendars => (Vec::new(), "no calendars".to_string()),
        Reading::Read(objects) => {
            let from = midnight_of(now, zone);
            let days = doca_ipc::calendar_ahead::days_after_today(&settings.ahead);
            let found = agenda::between(objects, from, from + Span::days(days + 1), zone);
            let upcoming = in_order(&found, now, zone)
                .into_iter()
                .filter(|event| settings.all_day || !event.all_day)
                .take(UPCOMING)
                .map(|event| Upcoming {
                    when: when(event, now, zone),
                    title: if event.title.is_empty() { "(no title)".to_string() } else { event.title.clone() },
                    now: event.is_on(now) && !event.all_day,
                })
                .collect();
            (upcoming, String::new())
        }
    };

    WidgetState::new(
        "calendar",
        Body::Calendar(doca_ipc::Calendar {
            weekday: weekday(today.weekday()).to_string(),
            day: today.day(),
            upcoming,
            missing,
        }),
    )
}

/// What has not ended, in the order it is worth reading.
///
/// Soonest first — except that today's all-day events go after today's
/// timed ones. A birthday frames the day; the 14:00 is the thing you need to
/// know about, and when the bar is crowded the tile keeps only the first
/// line, so the first line has to be the one that matters.
fn in_order(found: &[Occurrence], now: DateTime<Utc>, zone: Tz) -> Vec<&Occurrence> {
    let tomorrow = midnight_of(now, zone) + Span::days(1);
    let mut open: Vec<&Occurrence> = found
        .iter()
        .filter(|o| o.end > now || (o.start == o.end && o.start >= now))
        .collect();
    // Stable, so everything else keeps the agenda's own order.
    open.sort_by_key(|o| {
        let framing_today = o.all_day && o.start < tomorrow;
        let day = if o.start < tomorrow { 0 } else { 1 };
        (day, framing_today, if o.start < tomorrow { now } else { o.start })
    });
    open
}

/// When, in as few characters as a tile has: `now`, `19:00`, `today`,
/// `Tue 14:00`, `Tue`.
fn when(event: &Occurrence, now: DateTime<Utc>, zone: Tz) -> String {
    let start = event.start.with_timezone(&zone);
    let tomorrow = midnight_of(now, zone) + Span::days(1);
    match (event.all_day, event.start < tomorrow) {
        (false, _) if event.is_on(now) => "now".to_string(),
        (true, true) => "today".to_string(),
        (false, true) => start.format("%H:%M").to_string(),
        (true, false) => weekday(start.weekday()).to_string(),
        (false, false) => format!("{} {}", weekday(start.weekday()), start.format("%H:%M")),
    }
}

fn weekday(day: chrono::Weekday) -> &'static str {
    ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"][day.num_days_from_monday() as usize]
}

fn midnight_of(now: DateTime<Utc>, zone: Tz) -> DateTime<Utc> {
    let day = now.with_timezone(&zone).date_naive();
    zone.from_local_datetime(&day.and_time(NaiveTime::MIN))
        .earliest()
        .map(|at| at.with_timezone(&Utc))
        .unwrap_or(now)
}

/// The zone the user is in, by name.
///
/// By name because a name carries the daylight-saving rules and an offset
/// does not: `Local` says -03:00 today and nothing about next March. The
/// places a Linux system says it are tried in the order glibc reads them.
pub fn local_zone() -> Tz {
    let named = std::env::var("TZ")
        .ok()
        .map(|tz| tz.trim_start_matches(':').to_string())
        .or_else(|| std::fs::read_to_string("/etc/timezone").ok())
        .or_else(|| {
            std::fs::read_link("/etc/localtime").ok().and_then(|path| {
                let path = path.to_string_lossy().into_owned();
                path.split_once("zoneinfo/").map(|(_, name)| name.to_string())
            })
        });
    if let Some(zone) = named.and_then(|name| name.trim().parse::<Tz>().ok()) {
        return zone;
    }
    // Every Linux this runs on says it one of the three ways above; UTC is
    // for the one that does not, and it is said out loud rather than guessed.
    tracing::warn!("cannot tell which zone this is, reading the calendar in UTC");
    Tz::UTC
}

/// Read the calendars every [`REFRESH`], for as long as anyone listens.
fn read_forever(zone: Tz, tx: mpsc::Sender<Reading>) {
    let mut eds = Eds::default();
    loop {
        let reading = eds.read(Utc::now(), zone);
        if tx.send(reading).is_err() {
            return;
        }
        std::thread::sleep(REFRESH);
    }
}

/// The connection to EDS and the calendars already opened through it.
#[derive(Default)]
struct Eds {
    connection: Option<zbus::blocking::Connection>,
    /// Source id to `(bus name, object path)`, so a calendar is opened once
    /// rather than every minute.
    opened: HashMap<String, (String, String)>,
    /// What has already been said, so a calendar that cannot be read is
    /// logged once and not every minute.
    complained: HashSet<String>,
}

impl Eds {
    fn read(&mut self, now: DateTime<Utc>, zone: Tz) -> Reading {
        if self.connection.is_none() {
            self.connection = zbus::blocking::Connection::session().ok();
        }
        let Some(connection) = self.connection.clone() else {
            self.once("bus", "no session bus to read the calendar from");
            return Reading::NoService;
        };

        let sources = match calendar_sources(&connection) {
            Ok(sources) => sources,
            Err(e) => {
                self.once("service", &format!("no calendar service: {e}"));
                return Reading::NoService;
            }
        };
        if sources.is_empty() {
            return Reading::NoCalendars;
        }

        let from = midnight_of(now, zone);
        let query = format!(
            "(occur-in-time-range? (make-time \"{}\") (make-time \"{}\"))",
            from.format("%Y%m%dT%H%M%SZ"),
            (from + Span::days(AHEAD + 1)).format("%Y%m%dT%H%M%SZ"),
        );
        let mut objects = Vec::new();
        for uid in sources {
            match self.objects(&connection, &uid, &query) {
                Ok(found) => objects.extend(found),
                Err(e) => {
                    // Opened again next time: the backend may have restarted.
                    self.opened.remove(&uid);
                    self.once(&uid, &format!("calendar {uid} could not be read: {e}"));
                }
            }
        }
        Reading::Read(objects)
    }

    fn objects(&mut self, connection: &zbus::blocking::Connection, uid: &str, query: &str) -> zbus::Result<Vec<String>> {
        if !self.opened.contains_key(uid) {
            let factory = zbus::blocking::Proxy::new(connection, CALENDARS, CALENDAR_FACTORY, FACTORY_INTERFACE)?;
            let (path, bus): (String, String) = factory.call("OpenCalendar", &(uid,))?;
            let calendar = zbus::blocking::Proxy::new(connection, bus.clone(), path.clone(), CALENDAR_INTERFACE)?;
            let _: Vec<String> = calendar.call("Open", &())?;
            self.opened.insert(uid.to_string(), (bus, path));
        }
        let (bus, path) = &self.opened[uid];
        let calendar = zbus::blocking::Proxy::new(connection, bus.clone(), path.clone(), CALENDAR_INTERFACE)?;
        calendar.call("GetObjectList", &(query,))
    }

    fn once(&mut self, what: &str, message: &str) {
        if self.complained.insert(what.to_string()) {
            tracing::warn!("{message}");
        }
    }
}

/// The ids of the calendars that are switched on.
fn calendar_sources(connection: &zbus::blocking::Connection) -> zbus::Result<Vec<String>> {
    let manager = zbus::blocking::fdo::ObjectManagerProxy::builder(connection)
        .destination(SOURCES)?
        .path(SOURCE_MANAGER)?
        .build()?;
    let mut sources: Vec<String> = manager
        .get_managed_objects()?
        .into_values()
        .filter_map(|interfaces| {
            let source = interfaces
                .into_iter()
                .find(|(name, _)| name.as_str() == SOURCE_INTERFACE)?
                .1;
            let uid = String::try_from(source.get("UID")?.clone()).ok()?;
            let data = String::try_from(source.get("Data")?.clone()).ok()?;
            is_shown_calendar(&data).then_some(uid)
        })
        .collect();
    sources.sort();
    Ok(sources)
}

/// Whether a source's key file describes a calendar the user has on.
///
/// `Enabled` is the account being on at all; `Selected`, in the `[Calendar]`
/// group, is the tick beside the calendar in GNOME Calendar. Both default to
/// on when they are not written, which is what EDS itself does.
pub fn is_shown_calendar(data: &str) -> bool {
    let mut group = "";
    let mut calendar = false;
    let mut on = true;
    for line in data.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix('[').and_then(|line| line.strip_suffix(']')) {
            group = name;
            calendar |= group == "Calendar";
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let off = value.trim().eq_ignore_ascii_case("false");
        match (group, key.trim()) {
            ("Data Source", "Enabled") | ("Calendar", "Selected") if off => on = false,
            _ => {}
        }
    }
    calendar && on
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAO_PAULO: Tz = chrono_tz::America::Sao_Paulo;

    fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
    }

    fn event(title: &str, start: &str, end: &str) -> String {
        format!("BEGIN:VEVENT\r\nUID:{title}\r\nSUMMARY:{title}\r\nDTSTART:{start}\r\nDTEND:{end}\r\nEND:VEVENT\r\n")
    }

    fn all_day(title: &str, day: &str) -> String {
        format!("BEGIN:VEVENT\r\nUID:{title}\r\nSUMMARY:{title}\r\nDTSTART;VALUE=DATE:{day}\r\nEND:VEVENT\r\n")
    }

    // Saturday 17 October 2026, 10:00 in São Paulo.
    fn saturday_morning() -> DateTime<Utc> {
        utc(2026, 10, 17, 13, 0)
    }

    fn calendar(state: &WidgetState) -> doca_ipc::Calendar {
        match state.body().expect("a body that reads back") {
            Body::Calendar(calendar) => calendar,
            other => panic!("a calendar drew {other:?}"),
        }
    }

    /// Each upcoming thing as the tile would put it on one line.
    fn lines(reading: &Reading, at: DateTime<Utc>) -> Vec<String> {
        calendar(&state_at(reading, at, SAO_PAULO, &CalendarSettings::default()))
            .upcoming
            .iter()
            .map(|up| format!("{} {}", up.when, up.title))
            .collect()
    }

    #[test]
    fn the_tile_says_the_day_and_what_is_next_with_its_time() {
        let reading = Reading::Read(vec![event("Lunch", "20261017T150000Z", "20261017T160000Z")]);
        let state = calendar(&state_at(&reading, saturday_morning(), SAO_PAULO, &CalendarSettings::default()));
        assert_eq!((state.weekday.as_str(), state.day), ("Sat", 17));
        assert_eq!(lines(&reading, saturday_morning()), vec!["12:00 Lunch"], "the time is the local one");
        assert!(!state.upcoming[0].now);
    }

    #[test]
    fn something_happening_now_is_told_apart_from_something_to_come() {
        let reading = Reading::Read(vec![
            event("Call", "20261017T124500Z", "20261017T133000Z"),
            event("Lunch", "20261017T150000Z", "20261017T160000Z"),
        ]);
        let state = calendar(&state_at(&reading, saturday_morning(), SAO_PAULO, &CalendarSettings::default()));
        assert_eq!(lines(&reading, saturday_morning()), vec!["now Call", "12:00 Lunch"]);
        assert_eq!(state.upcoming.iter().map(|up| up.now).collect::<Vec<_>>(), vec![true, false]);
    }

    #[test]
    fn an_all_day_event_does_not_hide_the_meeting_later_that_day() {
        let reading = Reading::Read(vec![
            all_day("Birthday", "20261017"),
            event("Dinner", "20261017T220000Z", "20261017T233000Z"),
            event("Mass", "20261018T120000Z", "20261018T130000Z"),
        ]);
        assert_eq!(
            lines(&reading, saturday_morning()),
            vec!["19:00 Dinner", "today Birthday", "Sun 09:00 Mass"],
            "the first line is the one a crowded bar keeps, so it is the next timed thing"
        );
    }

    #[test]
    fn what_is_past_is_not_listed_and_later_days_say_which() {
        let reading = Reading::Read(vec![
            event("Breakfast", "20261017T100000Z", "20261017T110000Z"),
            event("Dentist", "20261020T170000Z", "20261020T180000Z"),
            all_day("Holiday", "20261022"),
        ]);
        assert_eq!(lines(&reading, saturday_morning()), vec!["Tue 14:00 Dentist", "Thu Holiday"]);
    }

    #[test]
    fn nothing_ahead_and_nothing_to_read_are_not_the_same_thing() {
        let at = saturday_morning();
        let read = calendar(&state_at(&Reading::Read(vec![]), at, SAO_PAULO, &CalendarSettings::default()));
        assert!(read.upcoming.is_empty() && read.missing.is_empty(), "read, and nothing ahead");
        assert_eq!(calendar(&state_at(&Reading::NoService, at, SAO_PAULO, &CalendarSettings::default())).missing, "no calendar service");
        assert_eq!(calendar(&state_at(&Reading::NoCalendars, at, SAO_PAULO, &CalendarSettings::default())).missing, "no calendars");
        // The date does not wait on any of it.
        assert_eq!(calendar(&state_at(&Reading::Waiting, at, SAO_PAULO, &CalendarSettings::default())).day, 17);
    }

    #[test]
    fn the_date_turns_at_local_midnight_not_utc_midnight() {
        // 02:59 UTC on the 18th is still 23:59 on the 17th in São Paulo.
        let before = calendar(&state_at(&Reading::Waiting, utc(2026, 10, 18, 2, 59), SAO_PAULO, &CalendarSettings::default()));
        let after = calendar(&state_at(&Reading::Waiting, utc(2026, 10, 18, 3, 0), SAO_PAULO, &CalendarSettings::default()));
        assert_eq!((before.weekday.as_str(), before.day), ("Sat", 17));
        assert_eq!((after.weekday.as_str(), after.day), ("Sun", 18));
    }

    #[test]
    fn how_far_ahead_and_whether_all_day_are_the_users_to_say() {
        let reading = Reading::Read(vec![
            all_day("Birthday", "20261017"),
            event("Dinner", "20261017T220000Z", "20261017T233000Z"),
            event("Mass", "20261018T120000Z", "20261018T130000Z"),
            event("Dentist", "20261022T170000Z", "20261022T180000Z"),
            event("Trip", "20261105T120000Z", "20261105T130000Z"),
        ]);
        let with = |ahead: &str, all_day: bool| -> Vec<String> {
            let settings = CalendarSettings { ahead: ahead.to_string(), all_day };
            calendar(&state_at(&reading, saturday_morning(), SAO_PAULO, &settings))
                .upcoming
                .iter()
                .map(|up| up.title.clone())
                .collect()
        };
        assert_eq!(with("today", true), vec!["Dinner", "Birthday"]);
        assert_eq!(with("3-days", true), vec!["Dinner", "Birthday", "Mass"]);
        assert_eq!(with("week", true), vec!["Dinner", "Birthday", "Mass", "Dentist"]);
        assert_eq!(with("month", true), vec!["Dinner", "Birthday", "Mass", "Dentist", "Trip"]);
        assert_eq!(with("week", false), vec!["Dinner", "Mass", "Dentist"], "all-day left out");
        assert_eq!(with("fortnight", true), with("week", true), "an unknown choice is the default");
    }

    #[test]
    fn a_week_full_of_things_sends_only_what_could_be_shown() {
        let many: Vec<String> = (10..30)
            .map(|day| event(&format!("e{day}"), &format!("202610{day}T150000Z"), &format!("202610{day}T160000Z")))
            .collect();
        let month = CalendarSettings { ahead: "month".to_string(), all_day: true };
        let state = calendar(&state_at(&Reading::Read(many), saturday_morning(), SAO_PAULO, &month));
        assert_eq!(state.upcoming.len(), UPCOMING);
    }

    /// Against the calendars on this machine, through the EDS really
    /// running: every object EDS hands back is understood, and some of what
    /// it holds lands in the week. Counts only, never a title.
    ///
    /// ```text
    /// cargo test -p docad -- --ignored reads_the_calendars_on_this_machine --nocapture
    /// ```
    #[test]
    #[ignore = "reads the session's own Evolution Data Server"]
    fn reads_the_calendars_on_this_machine() {
        let zone = local_zone();
        let now = Utc::now();
        let Reading::Read(objects) = Eds::default().read(now, zone) else {
            panic!("no calendars to read on this session");
        };
        let unread: Vec<usize> = objects
            .iter()
            .enumerate()
            .filter(|(_, object)| agenda::between(&[(*object).clone()], now - Span::days(3650), now + Span::days(3650), zone).is_empty())
            .map(|(at, _)| at)
            .collect();
        let from = midnight_of(now, zone);
        let week = agenda::between(&objects, from, from + Span::days(AHEAD + 1), zone);
        println!(
            "zone {zone}: {} objects, {} occurrences this week, {} all-day, {} not placed in time",
            objects.len(),
            week.len(),
            week.iter().filter(|o| o.all_day).count(),
            unread.len()
        );
        assert!(unread.is_empty(), "objects {unread:?} were handed over and not understood");
    }

    #[test]
    fn only_calendars_that_are_switched_on_are_read() {
        let google = "[Data Source]\nDisplayName=Work\nEnabled=true\nParent=abc\n\n[Calendar]\nBackendName=caldav\nSelected=true\n";
        assert!(is_shown_calendar(google));
        assert!(is_shown_calendar("[Data Source]\nDisplayName=Personal\n\n[Calendar]\nBackendName=local\n"), "unwritten means on");
        assert!(!is_shown_calendar(&google.replace("Selected=true", "Selected=false")), "unticked in GNOME Calendar");
        assert!(!is_shown_calendar(&google.replace("Enabled=true", "Enabled=false")), "account switched off");
        assert!(!is_shown_calendar("[Data Source]\nDisplayName=Tasks\n\n[Task List]\nBackendName=local\n"), "not a calendar");
    }
}
