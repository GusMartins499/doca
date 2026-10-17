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
use doca_ipc::{Body, Simple, WidgetState, NO_PROGRESS};

use super::agenda::{self, Occurrence};
use super::Widget;

const SOURCES: &str = "org.gnome.evolution.dataserver.Sources5";
const SOURCE_MANAGER: &str = "/org/gnome/evolution/dataserver/SourceManager";
const SOURCE_INTERFACE: &str = "org.gnome.evolution.dataserver.Source";
const CALENDARS: &str = "org.gnome.evolution.dataserver.Calendar8";
const CALENDAR_FACTORY: &str = "/org/gnome/evolution/dataserver/CalendarFactory";
const FACTORY_INTERFACE: &str = "org.gnome.evolution.dataserver.CalendarFactory";
const CALENDAR_INTERFACE: &str = "org.gnome.evolution.dataserver.Calendar";

/// How far ahead the tile looks for "next".
const AHEAD: i64 = 7;

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
    zone: Tz,
    reading: Reading,
    readings: Option<mpsc::Receiver<Reading>>,
}

impl Calendar {
    pub fn new() -> Self {
        let zone = local_zone();
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("calendar".into())
            .spawn(move || read_forever(zone, tx));
        if let Err(e) = spawned {
            tracing::warn!("cannot start reading the calendar: {e}");
        }
        Self { zone, reading: Reading::Waiting, readings: Some(rx) }
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
        state_at(&self.reading, Utc::now(), self.zone)
    }
}

/// What the tile shows at `now`, given what was read.
pub fn state_at(reading: &Reading, now: DateTime<Utc>, zone: Tz) -> WidgetState {
    let today = now.with_timezone(&zone);
    let label = format!("{} {}", weekday(today.weekday()), today.day());

    let (detail, active) = match reading {
        Reading::Waiting => (String::new(), false),
        Reading::NoService => ("no calendar service".to_string(), false),
        Reading::NoCalendars => ("no calendars".to_string(), false),
        Reading::Read(objects) => {
            let from = midnight_of(now, zone);
            let found = agenda::between(objects, from, from + Span::days(AHEAD + 1), zone);
            match next(&found, now, zone) {
                Some(event) => (describe(event, now, zone), event.is_on(now)),
                None => ("nothing ahead".to_string(), false),
            }
        }
    };

    WidgetState::new(
        "calendar",
        Body::Simple(Simple { label, detail, progress: NO_PROGRESS, active }),
    )
}

/// The one occurrence the tile is about.
///
/// What has not ended, soonest first — except that an all-day event does
/// not hide a meeting later the same day. A birthday frames the day; the
/// 14:00 is the thing you need to know about. All-day is shown when nothing
/// timed is left today.
fn next(found: &[Occurrence], now: DateTime<Utc>, zone: Tz) -> Option<&Occurrence> {
    let open: Vec<&Occurrence> = found.iter().filter(|o| o.end > now || (o.start == o.end && o.start >= now)).collect();
    let tomorrow = midnight_of(now, zone) + Span::days(1);
    open.iter()
        .find(|o| !o.all_day && o.start < tomorrow)
        .or_else(|| open.first())
        .copied()
}

/// One line for the tile: when, then what.
fn describe(event: &Occurrence, now: DateTime<Utc>, zone: Tz) -> String {
    let title = if event.title.is_empty() { "(no title)" } else { event.title.as_str() };
    if event.is_on(now) && !event.all_day {
        return format!("now · {title}");
    }
    let start = event.start.with_timezone(&zone);
    let today = midnight_of(now, zone);
    let day = if event.start < today + Span::days(1) {
        String::new()
    } else if event.start < today + Span::days(2) {
        "tomorrow ".to_string()
    } else {
        format!("{} ", weekday(start.weekday()))
    };
    if event.all_day {
        format!("{day}{title}").trim_start().to_string()
    } else {
        format!("{day}{} {title}", start.format("%H:%M"))
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
    use super::super::Drawn;
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

    #[test]
    fn the_tile_says_the_day_and_what_is_next_with_its_time() {
        let reading = Reading::Read(vec![event("Lunch", "20261017T150000Z", "20261017T160000Z")]);
        let state = state_at(&reading, saturday_morning(), SAO_PAULO);
        assert_eq!(state.label(), "Sat 17");
        assert_eq!(state.detail(), "12:00 Lunch", "the time is the local one");
        assert!(!state.active());
    }

    #[test]
    fn something_happening_now_is_told_apart_from_something_to_come() {
        let reading = Reading::Read(vec![event("Call", "20261017T124500Z", "20261017T133000Z")]);
        let state = state_at(&reading, saturday_morning(), SAO_PAULO);
        assert_eq!(state.detail(), "now · Call");
        assert!(state.active(), "an event under way is not drawn like one to come");
    }

    #[test]
    fn an_all_day_event_does_not_hide_the_meeting_later_that_day() {
        let reading = Reading::Read(vec![
            all_day("Birthday", "20261017"),
            event("Dinner", "20261017T220000Z", "20261017T233000Z"),
        ]);
        assert_eq!(state_at(&reading, saturday_morning(), SAO_PAULO).detail(), "19:00 Dinner");

        let only = Reading::Read(vec![all_day("Birthday", "20261017")]);
        assert_eq!(state_at(&only, saturday_morning(), SAO_PAULO).detail(), "Birthday");
    }

    #[test]
    fn what_is_past_is_not_next_and_later_days_say_which() {
        let reading = Reading::Read(vec![
            event("Breakfast", "20261017T100000Z", "20261017T110000Z"),
            event("Mass", "20261018T120000Z", "20261018T130000Z"),
        ]);
        assert_eq!(state_at(&reading, saturday_morning(), SAO_PAULO).detail(), "tomorrow 09:00 Mass");

        let later = Reading::Read(vec![event("Dentist", "20261020T170000Z", "20261020T180000Z")]);
        assert_eq!(state_at(&later, saturday_morning(), SAO_PAULO).detail(), "Tue 14:00 Dentist");
    }

    #[test]
    fn nothing_ahead_and_nothing_to_read_say_so_rather_than_drawing_nothing() {
        let at = saturday_morning();
        assert_eq!(state_at(&Reading::Read(vec![]), at, SAO_PAULO).detail(), "nothing ahead");
        assert_eq!(state_at(&Reading::NoService, at, SAO_PAULO).detail(), "no calendar service");
        assert_eq!(state_at(&Reading::NoCalendars, at, SAO_PAULO).detail(), "no calendars");
        // The date does not wait on any of it.
        assert_eq!(state_at(&Reading::Waiting, at, SAO_PAULO).label(), "Sat 17");
    }

    #[test]
    fn the_date_turns_at_local_midnight_not_utc_midnight() {
        // 02:59 UTC on the 18th is still 23:59 on the 17th in São Paulo.
        let before = state_at(&Reading::Waiting, utc(2026, 10, 18, 2, 59), SAO_PAULO);
        let after = state_at(&Reading::Waiting, utc(2026, 10, 18, 3, 0), SAO_PAULO);
        assert_eq!((before.label().as_str(), after.label().as_str()), ("Sat 17", "Sun 18"));
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
