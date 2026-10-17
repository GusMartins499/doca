//! What is on the calendar between two instants, from iCalendar text.
//!
//! Everything here is pure: text in, occurrences out, with the local zone
//! handed in rather than looked up. The calendar widget is the one that
//! talks to a calendar service; this is the part of it that can be wrong in
//! ways a test can catch, which is most of it — the issue (#31) names the
//! places every calendar widget breaks, and they are all in this file:
//! time zones and daylight saving, recurrence, and the exceptions to it.
//!
//! # What is read
//!
//! `VEVENT`s only, and of those: `UID`, `SUMMARY`, `STATUS`, `DTSTART`,
//! `DTEND` or `DURATION`, `RRULE`, `EXDATE` and `RECURRENCE-ID`. That list is
//! what the calendars on the machine this was written on actually carry —
//! Google calendars through GNOME Online Accounts, read from Evolution Data
//! Server — measured before a line of this was written: all-day dates,
//! instants in UTC, instants in a named zone, `BYDAY=2MO`, `INTERVAL=2`,
//! `BYMONTHDAY`, a deleted occurrence and moved ones. Alarms, attendees and
//! descriptions are skipped without being read: this widget counts what is
//! next, and nothing it does not draw is worth parsing.
//!
//! `RDATE` is not read. None of the calendars measured used it, and an
//! occurrence it would add is one this widget misses rather than one it
//! invents.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use chrono_tz::Tz;
use rrule::{RRule, Unvalidated};

/// One time an event happens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occurrence {
    pub title: String,
    /// Takes up a whole day or several, rather than a time within one.
    pub all_day: bool,
    /// For an all-day event, midnight of its first day where the user is: a
    /// birthday is on the 17th wherever the calendar that holds it was made.
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
}

impl Occurrence {
    /// Under way at this instant.
    pub fn is_on(&self, now: DateTime<Utc>) -> bool {
        self.start <= now && now < self.end
    }
}

/// The most occurrences one series may put into a window. A daily event in
/// a month is thirty-one; this is a ceiling on a malformed rule, not a limit
/// anyone should meet.
const PER_SERIES: u16 = 512;

/// Every occurrence that overlaps `[from, to)`, soonest first.
///
/// `objects` is iCalendar text as a calendar service hands it over — one
/// string per object, each holding one or more `VEVENT`s — and `local` is
/// the zone the user is in, which is what a time with no zone and an all-day
/// date are read in.
pub fn between(
    objects: &[String],
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    local: Tz,
) -> Vec<Occurrence> {
    let events: Vec<Event> = objects
        .iter()
        .flat_map(|object| events_in(object))
        .collect();

    // A moved or cancelled occurrence is its own `VEVENT`, with the series'
    // UID and a RECURRENCE-ID naming which occurrence it replaces. The series
    // then must not also produce that one: it is either somewhere else now,
    // or gone.
    let mut replaced: HashMap<&str, HashSet<Key>> = HashMap::new();
    for event in &events {
        if let Some(original) = &event.recurrence_id {
            replaced
                .entry(event.uid.as_str())
                .or_default()
                .insert(original.key(local));
        }
    }

    let mut found = Vec::new();
    for event in &events {
        if event.cancelled {
            continue;
        }
        let starts = match (&event.rule, &event.recurrence_id) {
            (Some(rule), None) => {
                let skip = replaced.get(event.uid.as_str());
                expand(event, rule, from, to, local)
                    .into_iter()
                    .filter(|start| {
                        let key = start.key(local);
                        !skip.is_some_and(|skip| skip.contains(&key))
                            && !event.exceptions.iter().any(|except| except.key(local) == key)
                    })
                    .collect()
            }
            _ => vec![event.start.clone()],
        };
        for start in starts {
            let occurrence = event.at(&start, local);
            if overlaps(&occurrence, from, to) {
                found.push(occurrence);
            }
        }
    }

    // All-day first among things that begin at the same instant: it frames
    // the day, and the meeting at midnight is the rarer of the two.
    found.sort_by(|a, b| {
        (a.start, !a.all_day, &a.title).cmp(&(b.start, !b.all_day, &b.title))
    });
    found
}

/// Whether an occurrence touches the window. An instant with no length is in
/// it when it starts there; anything longer, when any of it is.
fn overlaps(occurrence: &Occurrence, from: DateTime<Utc>, to: DateTime<Utc>) -> bool {
    if occurrence.start == occurrence.end {
        return from <= occurrence.start && occurrence.start < to;
    }
    occurrence.start < to && occurrence.end > from
}

/// A point in time as the calendar wrote it, before anyone decides what it
/// means.
#[derive(Debug, Clone, PartialEq)]
enum Stamp {
    /// `VALUE=DATE`: a day, not an instant.
    Date(NaiveDate),
    /// Ends in `Z`.
    Utc(NaiveDateTime),
    /// Has a `TZID` this build knows.
    Zoned(NaiveDateTime, Tz),
    /// Neither: whatever the clock on the wall says, wherever the wall is.
    Floating(NaiveDateTime),
}

/// What two stamps are compared by when one replaces or excludes the other.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum Key {
    Day(NaiveDate),
    At(DateTime<Utc>),
}

impl Stamp {
    fn instant(&self, local: Tz) -> DateTime<Utc> {
        match self {
            Stamp::Date(day) => midnight(*day, local),
            Stamp::Utc(at) => Utc.from_utc_datetime(at),
            Stamp::Zoned(at, zone) => in_zone(at, *zone),
            Stamp::Floating(at) => in_zone(at, local),
        }
    }

    fn key(&self, local: Tz) -> Key {
        match self {
            Stamp::Date(day) => Key::Day(*day),
            _ => Key::At(self.instant(local)),
        }
    }

    fn is_date(&self) -> bool {
        matches!(self, Stamp::Date(_))
    }
}

/// A wall-clock time in a zone, as one instant.
///
/// The two hard cases are the hour daylight saving skips and the hour it
/// repeats. A time in the repeated hour is the first of the two, which is
/// what calendars mean by it. A time in the skipped hour does not exist; it
/// is read an hour later, where the clock went, rather than dropped — an
/// event the calendar holds is not one to lose over a rule change.
fn in_zone(at: &NaiveDateTime, zone: Tz) -> DateTime<Utc> {
    zone.from_local_datetime(at)
        .earliest()
        .or_else(|| zone.from_local_datetime(&(*at + Duration::hours(1))).earliest())
        .map(|at| at.with_timezone(&Utc))
        .unwrap_or_else(|| Utc.from_utc_datetime(at))
}

fn midnight(day: NaiveDate, zone: Tz) -> DateTime<Utc> {
    in_zone(&day.and_time(NaiveTime::MIN), zone)
}

/// One `VEVENT`, as far as this widget cares.
#[derive(Debug, Clone, PartialEq)]
struct Event {
    uid: String,
    title: String,
    start: Stamp,
    length: Length,
    rule: Option<String>,
    exceptions: Vec<Stamp>,
    recurrence_id: Option<Stamp>,
    cancelled: bool,
}

/// How long an event lasts: whole days for an all-day one, so that a day on
/// which the clocks change is still one day long.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Length {
    Days(i64),
    Exactly(Duration),
}

impl Event {
    /// This event, happening at `start`.
    fn at(&self, start: &Stamp, local: Tz) -> Occurrence {
        let begins = start.instant(local);
        let ends = match (self.length, start) {
            (Length::Days(days), Stamp::Date(day)) => midnight(*day + Duration::days(days), local),
            (Length::Days(days), _) => begins + Duration::days(days),
            (Length::Exactly(length), _) => begins + length,
        };
        Occurrence {
            title: self.title.clone(),
            all_day: start.is_date(),
            start: begins,
            end: ends.max(begins),
        }
    }
}

/// The starts a series has in the window, as stamps of the same kind as its
/// first one — so an all-day series yields dates and the rest instants.
fn expand(event: &Event, rule: &str, from: DateTime<Utc>, to: DateTime<Utc>, local: Tz) -> Vec<Stamp> {
    // The crate validates UNTIL against DTSTART's zone and wants it in UTC
    // whenever the start has a zone, while calendars write it in whatever
    // they please — a bare date for an all-day series, UTC for the rest. So
    // UNTIL is taken out of the rule, read here, and handed over in the one
    // form that is always accepted.
    let mut until = None;
    let rest: Vec<&str> = rule
        .split(';')
        .filter(|part| match part.strip_prefix("UNTIL=") {
            Some(value) => {
                until = stamp_value(value, None);
                false
            }
            None => true,
        })
        .collect();
    let Ok(parsed) = rest.join(";").parse::<RRule<Unvalidated>>() else {
        // A rule this cannot read is shown once, at its own start, rather
        // than every day or never.
        return vec![event.start.clone()];
    };

    // An all-day series is walked in UTC, where every day is 24 hours long,
    // and read back as dates; the rest in their own zone, so 09:00 stays
    // 09:00 across a change of clocks.
    let (zone, first) = match &event.start {
        Stamp::Date(day) => (Tz::UTC, day.and_time(NaiveTime::MIN)),
        Stamp::Utc(at) => (Tz::UTC, *at),
        Stamp::Zoned(at, zone) => (*zone, *at),
        Stamp::Floating(at) => (local, *at),
    };
    let zone = rrule::Tz::Tz(zone);
    let Some(first) = zone.from_local_datetime(&first).earliest() else {
        return vec![event.start.clone()];
    };

    let own_zone = match &event.start {
        Stamp::Zoned(_, zone) => *zone,
        Stamp::Date(_) | Stamp::Utc(_) => Tz::UTC,
        Stamp::Floating(_) => local,
    };
    let parsed = match until {
        Some(until) => {
            let last = match until {
                // A date is the whole of the day it names: an all-day
                // series walks midnights, so its midnight is enough, and a
                // timed one runs to the end of that day where it happens.
                Stamp::Date(day) if event.start.is_date() => midnight(day, Tz::UTC),
                Stamp::Date(day) => midnight(day + Duration::days(1), own_zone) - Duration::seconds(1),
                other => other.instant(own_zone),
            };
            parsed.until(last.with_timezone(&rrule::Tz::UTC))
        }
        None => parsed,
    };
    let Ok(set) = parsed.build(first) else {
        return vec![event.start.clone()];
    };

    // Started early enough to catch the one already under way when the
    // window opens.
    let reach = match event.length {
        Length::Days(days) => Duration::days(days + 1),
        Length::Exactly(length) => length,
    };
    let after = (from - reach).with_timezone(&rrule::Tz::UTC);
    let before = to.with_timezone(&rrule::Tz::UTC);
    let mut starts = set.after(after).before(before).all(PER_SERIES).dates;
    // RFC 5545: "The DTSTART property value always counts as the first
    // occurrence", even when the rule would not land on it — a weekly
    // Monday-Wednesday-Friday series first put on a Sunday happens that
    // Sunday too. rrule follows dateutil, which leaves it out; GNOME
    // Calendar and Evolution show it, so this does.
    if (after..=before).contains(&first) && !starts.contains(&first) {
        starts.insert(0, first);
    }
    starts
        .into_iter()
        .map(|start| match &event.start {
            Stamp::Date(_) => Stamp::Date(start.date_naive()),
            Stamp::Utc(_) => Stamp::Utc(start.naive_utc()),
            Stamp::Zoned(_, zone) => Stamp::Zoned(start.naive_local(), *zone),
            Stamp::Floating(_) => Stamp::Floating(start.naive_local()),
        })
        .collect()
}

/// A property line: `NAME;PARAM=value;…:value`.
#[derive(Debug, Clone, PartialEq)]
struct Property {
    name: String,
    params: Vec<(String, String)>,
    value: String,
}

impl Property {
    fn param(&self, name: &str) -> Option<&str> {
        self.params
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// Every event in one object's text that this widget can place in time.
fn events_in(text: &str) -> Vec<Event> {
    let mut events = Vec::new();
    let mut current: Option<Vec<Property>> = None;
    // Inside an alarm, or anything else nested in an event, nothing is the
    // event's: an alarm has a DESCRIPTION and a TRIGGER of its own.
    let mut nested = 0usize;

    for property in properties(text) {
        match (property.name.as_str(), property.value.to_ascii_uppercase().as_str()) {
            ("BEGIN", "VEVENT") => {
                current = Some(Vec::new());
                nested = 0;
            }
            ("END", "VEVENT") => {
                if let Some(event) = current.take().and_then(|props| event_from(&props)) {
                    events.push(event);
                }
            }
            ("BEGIN", _) if current.is_some() => nested += 1,
            ("END", _) if current.is_some() => nested = nested.saturating_sub(1),
            _ if nested == 0 => {
                if let Some(props) = current.as_mut() {
                    props.push(property);
                }
            }
            _ => {}
        }
    }
    events
}

fn event_from(props: &[Property]) -> Option<Event> {
    let find = |name: &str| props.iter().find(|p| p.name == name);
    let start_prop = find("DTSTART")?;
    let start = stamp(start_prop)?;

    let length = if let Some(end) = find("DTEND").and_then(stamp) {
        match (&start, &end) {
            (Stamp::Date(from), Stamp::Date(to)) => Length::Days((*to - *from).num_days().max(1)),
            // Mixed kinds are a broken event; any local zone reads them the
            // same way relative to each other, so UTC is as good as another.
            _ => Length::Exactly(end.instant(Tz::UTC) - start.instant(Tz::UTC)),
        }
    } else if let Some(duration) = find("DURATION").and_then(|p| duration(&p.value)) {
        if start.is_date() && duration.num_seconds() % 86_400 == 0 {
            Length::Days(duration.num_days().max(1))
        } else {
            Length::Exactly(duration)
        }
    } else if start.is_date() {
        // RFC 5545: an all-day event with no end is that one day.
        Length::Days(1)
    } else {
        Length::Exactly(Duration::zero())
    };

    let exceptions = props
        .iter()
        .filter(|p| p.name == "EXDATE")
        .flat_map(|p| {
            let zone = p.param("TZID");
            let date = p.param("VALUE").is_some_and(|v| v.eq_ignore_ascii_case("DATE"));
            p.value
                .split(',')
                .filter_map(move |value| stamp_of(value, zone, date))
                .collect::<Vec<_>>()
        })
        .collect();

    Some(Event {
        uid: find("UID").map(|p| p.value.clone()).unwrap_or_default(),
        title: find("SUMMARY").map(|p| unescape(&p.value)).unwrap_or_default(),
        start,
        length,
        rule: find("RRULE").map(|p| p.value.clone()),
        exceptions,
        recurrence_id: find("RECURRENCE-ID").and_then(stamp),
        cancelled: find("STATUS").is_some_and(|p| p.value.eq_ignore_ascii_case("CANCELLED")),
    })
}

fn stamp(property: &Property) -> Option<Stamp> {
    let date = property
        .param("VALUE")
        .is_some_and(|value| value.eq_ignore_ascii_case("DATE"));
    stamp_of(&property.value, property.param("TZID"), date)
}

fn stamp_of(value: &str, zone: Option<&str>, date: bool) -> Option<Stamp> {
    if date || value.len() == 8 {
        return NaiveDate::parse_from_str(value, "%Y%m%d").ok().map(Stamp::Date);
    }
    stamp_value(value, zone.map(zone_named))
}

/// A DATE or DATE-TIME value, in `zone` when it has one and none of its own.
///
/// A zone named but unknown is `Some(None)` and reads as floating: the wall
/// clock where the user is, which is right for every calendar made by
/// someone in the same place, and an hour or so off for the rest.
fn stamp_value(value: &str, zone: Option<Option<Tz>>) -> Option<Stamp> {
    if value.len() == 8 {
        return NaiveDate::parse_from_str(value, "%Y%m%d").ok().map(Stamp::Date);
    }
    if let Some(utc) = value.strip_suffix('Z') {
        return NaiveDateTime::parse_from_str(utc, "%Y%m%dT%H%M%S").ok().map(Stamp::Utc);
    }
    let at = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").ok()?;
    Some(match zone.flatten() {
        Some(zone) => Stamp::Zoned(at, zone),
        None => Stamp::Floating(at),
    })
}

/// A TZID as a zone, when it names one.
///
/// Most calendars write an IANA name. Some prefix one with where they got
/// their tables — `/freeassociation.sourceforge.net/Tzfile/Europe/London`,
/// `/mozilla.org/20050126_1/Europe/London` — and the name is still there at
/// the end, so leading segments are dropped until what is left is a zone.
fn zone_named(name: &str) -> Option<Tz> {
    let name = name.trim_matches('"');
    let mut rest = name;
    loop {
        if let Ok(zone) = rest.parse::<Tz>() {
            return Some(zone);
        }
        let (_, after) = rest.trim_start_matches('/').split_once('/')?;
        rest = after;
    }
}

/// An ISO 8601 duration as iCalendar writes them: `P1D`, `PT1H30M`, `P2W`.
fn duration(value: &str) -> Option<Duration> {
    let (negative, value) = match value.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, value.strip_prefix('+').unwrap_or(value)),
    };
    let value = value.strip_prefix('P')?;
    let mut total = Duration::zero();
    let mut number = String::new();
    let mut in_time = false;
    for c in value.chars() {
        match c {
            'T' => in_time = true,
            '0'..='9' => number.push(c),
            unit => {
                let n: i64 = number.parse().ok()?;
                number.clear();
                total += match (unit, in_time) {
                    ('W', false) => Duration::weeks(n),
                    ('D', false) => Duration::days(n),
                    ('H', true) => Duration::hours(n),
                    ('M', true) => Duration::minutes(n),
                    ('S', true) => Duration::seconds(n),
                    _ => return None,
                };
            }
        }
    }
    if !number.is_empty() {
        return None;
    }
    Some(if negative { -total } else { total })
}

/// Unfold the lines and split each into a property.
///
/// A line that begins with a space or a tab continues the one before it —
/// which is how a long SUMMARY arrives, cut wherever 75 octets fell.
fn properties(text: &str) -> Vec<Property> {
    let mut lines: Vec<String> = Vec::new();
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        match (line.strip_prefix(' ').or_else(|| line.strip_prefix('\t')), lines.last_mut()) {
            (Some(rest), Some(last)) => last.push_str(rest),
            _ => lines.push(line.to_string()),
        }
    }
    lines.iter().filter_map(|line| property(line)).collect()
}

/// `NAME;A=1;B="x:y":value` — the first colon outside quotes ends the
/// parameters, because a parameter may quote a colon.
fn property(line: &str) -> Option<Property> {
    let mut quoted = false;
    let split = line.char_indices().find(|(_, c)| {
        if *c == '"' {
            quoted = !quoted;
        }
        *c == ':' && !quoted
    })?;
    let (head, value) = (&line[..split.0], &line[split.0 + 1..]);

    let mut parts = head.split(';');
    let name = parts.next()?.trim().to_ascii_uppercase();
    if name.is_empty() {
        return None;
    }
    let params = parts
        .filter_map(|part| part.split_once('='))
        .map(|(key, value)| (key.to_ascii_uppercase(), value.trim_matches('"').to_string()))
        .collect();
    Some(Property { name, params, value: value.to_string() })
}

/// TEXT values escape `\`, `;`, `,` and newlines.
fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') | Some('N') => out.push(' '),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAO_PAULO: Tz = chrono_tz::America::Sao_Paulo;
    const NEW_YORK: Tz = chrono_tz::America::New_York;

    fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
    }

    fn object(body: &str) -> String {
        format!("BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\n{body}END:VEVENT\r\nEND:VCALENDAR\r\n")
    }

    fn event(lines: &[&str]) -> String {
        object(&lines.iter().map(|line| format!("{line}\r\n")).collect::<String>())
    }

    fn titles(found: &[Occurrence]) -> Vec<&str> {
        found.iter().map(|o| o.title.as_str()).collect()
    }

    fn starts(found: &[Occurrence]) -> Vec<DateTime<Utc>> {
        found.iter().map(|o| o.start).collect()
    }

    #[test]
    fn an_event_at_nine_in_its_zone_is_at_nine_in_that_zone() {
        let found = between(
            &[event(&[
                "UID:a",
                "SUMMARY:Standup",
                "DTSTART;TZID=America/Sao_Paulo:20261019T090000",
                "DTEND;TZID=America/Sao_Paulo:20261019T093000",
            ])],
            utc(2026, 10, 19, 0, 0),
            utc(2026, 10, 20, 0, 0),
            SAO_PAULO,
        );
        assert_eq!(starts(&found), vec![utc(2026, 10, 19, 12, 0)]);
        assert_eq!(found[0].end - found[0].start, Duration::minutes(30));
        assert!(!found[0].all_day);
    }

    /// The issue's own example: across a change of clocks, a weekly event at
    /// nine stays at nine on the wall, which is a different hour in UTC.
    #[test]
    fn a_weekly_event_keeps_its_hour_across_daylight_saving() {
        let found = between(
            &[event(&[
                "UID:a",
                "SUMMARY:Gym",
                "DTSTART;TZID=America/New_York:20261026T090000",
                "DTEND;TZID=America/New_York:20261026T100000",
                "RRULE:FREQ=WEEKLY;BYDAY=MO;WKST=SU",
            ])],
            utc(2026, 10, 26, 0, 0),
            utc(2026, 11, 3, 0, 0),
            SAO_PAULO,
        );
        // EDT then EST: 13:00 UTC, then 14:00 UTC — both 09:00 in New York.
        assert_eq!(starts(&found), vec![utc(2026, 10, 26, 13, 0), utc(2026, 11, 2, 14, 0)]);
    }

    /// The issue asked for exactly this before the widget could close: a
    /// weekly event with one occurrence deleted and one moved.
    #[test]
    fn a_weekly_event_with_an_exception_and_a_moved_occurrence() {
        let series = event(&[
            "UID:weekly",
            "SUMMARY:1:1",
            "DTSTART;TZID=America/Sao_Paulo:20261007T150000",
            "DTEND;TZID=America/Sao_Paulo:20261007T153000",
            "RRULE:FREQ=WEEKLY;BYDAY=WE;WKST=SU",
            "EXDATE;TZID=America/Sao_Paulo:20261014T150000",
        ]);
        let moved = event(&[
            "UID:weekly",
            "SUMMARY:1:1 (moved)",
            "RECURRENCE-ID;TZID=America/Sao_Paulo:20261021T150000",
            "DTSTART;TZID=America/Sao_Paulo:20261022T100000",
            "DTEND;TZID=America/Sao_Paulo:20261022T103000",
        ]);
        let found = between(
            &[series, moved],
            utc(2026, 10, 7, 0, 0),
            utc(2026, 10, 29, 0, 0),
            SAO_PAULO,
        );
        assert_eq!(
            titles(&found),
            vec!["1:1", "1:1 (moved)", "1:1"],
            "the 14th is deleted, the 21st moved to the 22nd, the 28th untouched"
        );
        assert_eq!(
            starts(&found),
            vec![utc(2026, 10, 7, 18, 0), utc(2026, 10, 22, 13, 0), utc(2026, 10, 28, 18, 0)]
        );
    }

    #[test]
    fn a_cancelled_occurrence_is_gone_and_a_cancelled_event_never_was() {
        let series = event(&[
            "UID:s",
            "SUMMARY:Daily",
            "DTSTART:20261019T120000Z",
            "RRULE:FREQ=DAILY",
        ]);
        let cancelled_one = event(&[
            "UID:s",
            "SUMMARY:Daily",
            "STATUS:CANCELLED",
            "RECURRENCE-ID:20261020T120000Z",
            "DTSTART:20261020T120000Z",
        ]);
        let cancelled = event(&["UID:c", "SUMMARY:Off", "STATUS:CANCELLED", "DTSTART:20261019T130000Z"]);
        let found = between(
            &[series, cancelled_one, cancelled],
            utc(2026, 10, 19, 0, 0),
            utc(2026, 10, 22, 0, 0),
            SAO_PAULO,
        );
        assert_eq!(starts(&found), vec![utc(2026, 10, 19, 12, 0), utc(2026, 10, 21, 12, 0)]);
    }

    #[test]
    fn an_all_day_event_is_the_whole_local_day_and_comes_first() {
        let found = between(
            &[
                event(&["UID:t", "SUMMARY:Midnight", "DTSTART:20261017T030000Z"]),
                event(&[
                    "UID:b",
                    "SUMMARY:Birthday",
                    "DTSTART;VALUE=DATE:20261017",
                    "DTEND;VALUE=DATE:20261018",
                ]),
            ],
            utc(2026, 10, 17, 0, 0),
            utc(2026, 10, 18, 12, 0),
            SAO_PAULO,
        );
        assert_eq!(titles(&found), vec!["Birthday", "Midnight"]);
        // Midnight in São Paulo, UTC-3, is three in the morning in UTC.
        assert_eq!(found[0].start, utc(2026, 10, 17, 3, 0));
        assert_eq!(found[0].end, utc(2026, 10, 18, 3, 0));
        assert!(found[0].all_day);
    }

    #[test]
    fn a_monthly_rule_on_the_second_monday_lands_on_it() {
        let found = between(
            &[event(&[
                "UID:m",
                "SUMMARY:Review",
                "DTSTART;TZID=America/Sao_Paulo:20260914T100000",
                "RRULE:FREQ=MONTHLY;BYDAY=2MO;WKST=SU",
            ])],
            utc(2026, 10, 1, 0, 0),
            utc(2026, 12, 31, 0, 0),
            SAO_PAULO,
        );
        let days: Vec<u32> = found
            .iter()
            .map(|o| chrono::Datelike::day(&o.start.with_timezone(&SAO_PAULO)))
            .collect();
        assert_eq!(days, vec![12, 9, 14]);
    }

    #[test]
    fn every_other_week_skips_the_week_between() {
        let found = between(
            &[event(&[
                "UID:x",
                "SUMMARY:Retro",
                "DTSTART;TZID=America/Sao_Paulo:20261002T160000",
                "RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=FR;WKST=SU",
            ])],
            utc(2026, 10, 1, 0, 0),
            utc(2026, 11, 1, 0, 0),
            SAO_PAULO,
        );
        let days: Vec<u32> = found
            .iter()
            .map(|o| chrono::Datelike::day(&o.start.with_timezone(&SAO_PAULO)))
            .collect();
        assert_eq!(days, vec![2, 16, 30]);
    }

    #[test]
    fn a_series_ends_where_its_until_says_whatever_form_it_takes() {
        for until in ["20261021T235959Z", "20261021"] {
            let found = between(
                &[event(&[
                    "UID:u",
                    "SUMMARY:Sprint",
                    "DTSTART;TZID=America/Sao_Paulo:20261019T100000",
                    &format!("RRULE:FREQ=DAILY;UNTIL={until}"),
                ])],
                utc(2026, 10, 1, 0, 0),
                utc(2026, 11, 1, 0, 0),
                SAO_PAULO,
            );
            assert_eq!(found.len(), 3, "UNTIL={until} gave {:?}", starts(&found));
        }

        let all_day = between(
            &[event(&[
                "UID:d",
                "SUMMARY:Trip",
                "DTSTART;VALUE=DATE:20261019",
                "RRULE:FREQ=DAILY;UNTIL=20261021",
            ])],
            utc(2026, 10, 1, 0, 0),
            utc(2026, 11, 1, 0, 0),
            SAO_PAULO,
        );
        assert_eq!(all_day.len(), 3);
    }

    #[test]
    fn the_first_day_of_a_series_counts_even_off_its_rule() {
        // Sunday the 18th, for a series that recurs on Mondays, Wednesdays
        // and Fridays.
        let found = between(
            &[event(&[
                "UID:g",
                "SUMMARY:Gym",
                "DTSTART;TZID=America/Sao_Paulo:20261018T100000",
                "RRULE:FREQ=WEEKLY;BYDAY=MO,WE,FR",
            ])],
            utc(2026, 10, 18, 0, 0),
            utc(2026, 10, 22, 0, 0),
            SAO_PAULO,
        );
        let days: Vec<u32> = found
            .iter()
            .map(|o| chrono::Datelike::day(&o.start.with_timezone(&SAO_PAULO)))
            .collect();
        assert_eq!(days, vec![18, 19, 21], "the Sunday it was put on, then the rule");
    }

    #[test]
    fn something_under_way_when_the_window_opens_is_in_it() {
        let found = between(
            &[event(&[
                "UID:l",
                "SUMMARY:Workshop",
                "DTSTART:20261019T120000Z",
                "DTEND:20261019T170000Z",
                "RRULE:FREQ=DAILY",
            ])],
            utc(2026, 10, 19, 15, 0),
            utc(2026, 10, 19, 16, 0),
            SAO_PAULO,
        );
        assert_eq!(starts(&found), vec![utc(2026, 10, 19, 12, 0)]);
        assert!(found[0].is_on(utc(2026, 10, 19, 15, 0)));
    }

    #[test]
    fn a_long_title_folded_across_lines_and_escaped_reads_whole() {
        let text = "BEGIN:VEVENT\r\nUID:f\r\nSUMMARY:Planning\\, round two\\; with the\r\n  whole team\r\nDTSTART:20261019T120000Z\r\nBEGIN:VALARM\r\nSUMMARY:not this one\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\nEND:VEVENT\r\n";
        let found = between(&[text.to_string()], utc(2026, 10, 19, 0, 0), utc(2026, 10, 20, 0, 0), SAO_PAULO);
        assert_eq!(titles(&found), vec!["Planning, round two; with the whole team"]);
    }

    #[test]
    fn a_zone_named_with_a_prefix_is_still_found_and_an_unknown_one_is_local() {
        assert_eq!(zone_named("/freeassociation.sourceforge.net/Tzfile/Europe/London"), Some(chrono_tz::Europe::London));
        assert_eq!(zone_named("\"America/Sao_Paulo\""), Some(SAO_PAULO));
        assert_eq!(zone_named("Pacific Standard Time"), None);

        let found = between(
            &[event(&["UID:w", "SUMMARY:Call", "DTSTART;TZID=Pacific Standard Time:20261019T090000"])],
            utc(2026, 10, 19, 0, 0),
            utc(2026, 10, 20, 0, 0),
            SAO_PAULO,
        );
        assert_eq!(starts(&found), vec![utc(2026, 10, 19, 12, 0)], "read as floating, in the user's zone");
    }

    #[test]
    fn a_time_in_the_hour_the_clocks_skip_is_read_after_it() {
        // New York skips 02:00–03:00 on 8 March 2026.
        let found = between(
            &[event(&["UID:g", "SUMMARY:Gap", "DTSTART;TZID=America/New_York:20260308T023000"])],
            utc(2026, 3, 8, 0, 0),
            utc(2026, 3, 9, 0, 0),
            NEW_YORK,
        );
        assert_eq!(starts(&found), vec![utc(2026, 3, 8, 7, 30)]);
    }

    #[test]
    fn durations_read_as_iso_8601_says() {
        assert_eq!(duration("PT1H30M"), Some(Duration::minutes(90)));
        assert_eq!(duration("P1D"), Some(Duration::days(1)));
        assert_eq!(duration("P2W"), Some(Duration::weeks(2)));
        assert_eq!(duration("-PT15M"), Some(Duration::minutes(-15)));
        assert_eq!(duration("P1Y"), None);
        assert_eq!(duration("PT5"), None);
    }

    #[test]
    fn a_rule_this_cannot_read_still_shows_the_event_once() {
        let found = between(
            &[event(&["UID:r", "SUMMARY:Odd", "DTSTART:20261019T120000Z", "RRULE:FREQ=FORTNIGHTLY"])],
            utc(2026, 10, 1, 0, 0),
            utc(2026, 11, 1, 0, 0),
            SAO_PAULO,
        );
        assert_eq!(starts(&found), vec![utc(2026, 10, 19, 12, 0)]);
    }
}
