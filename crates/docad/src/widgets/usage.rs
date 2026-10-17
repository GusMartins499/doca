//! What Claude Code has spent, counted from the files it already writes.
//!
//! # What is read, and why it might stop working
//!
//! Claude Code keeps one transcript per session as JSON Lines under
//! `$CLAUDE_CONFIG_DIR/projects/` — `~/.claude/projects/` when that is not
//! set — with subagents in a `subagents/` folder beside each one. Every reply
//! from the model is a line carrying `message.usage`. **That is an internal
//! format of another tool, not an API**: it can change in any release, and
//! when it does this is the file to look at. Measured on the machine this
//! was written on (142 files, 188 MB, two and a half months):
//!
//! - The same reply is written once per content block, with the same
//!   `message.id` and `requestId` and the same usage on every copy. Summing
//!   lines counted 143 where there were 82 replies. Replies are therefore
//!   counted once, by that pair.
//! - `stats-cache.json` holds daily totals but was last computed months
//!   before the files it summarises, so it is not used.
//! - Nothing on disk holds the plan's limits — see `claude.rs` for what that
//!   means for the tile.
//!
//! # What is kept
//!
//! From each line: the time, the two ids, and four numbers. The rest of the
//! line is the conversation; it is parsed past and dropped, never stored,
//! never logged, never sent anywhere. Nothing read here leaves the machine.
//!
//! # Reading without reading it all again
//!
//! A file is read from where the last read stopped, up to its last complete
//! line — a line still being written is left for next time. A file that got
//! shorter was replaced and is read from the start. The first read after
//! the daemon starts only opens files touched within [`KEEP`], because
//! nothing older can count towards any window this widget shows.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;

/// The longest window anything here is asked about, with a day to spare.
pub const KEEP: Duration = Duration::days(8);

/// Tokens, by the four kinds a reply reports.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tokens {
    pub input: u64,
    pub output: u64,
    pub cache_write: u64,
    pub cache_read: u64,
}

impl Tokens {
    /// What the tile counts: everything sent and received, and the cache
    /// written — but not the cache read back, which is the same tokens
    /// again. Read back they cost a fraction, and counting them would make
    /// the number on the tile mostly a measure of how long sessions are.
    pub fn spent(&self) -> u64 {
        self.input + self.output + self.cache_write
    }

    fn add(&mut self, other: &Tokens) {
        self.input += other.input;
        self.output += other.output;
        self.cache_write += other.cache_write;
        self.cache_read += other.cache_read;
    }
}

/// One reply, as far as this widget cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spend {
    pub at: DateTime<Utc>,
    pub tokens: Tokens,
}

/// What a read found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Found {
    /// There is no Claude Code folder at all.
    NoFolder,
    /// The folder is there, with no transcripts in it.
    Empty,
    /// Transcripts, and they were understood.
    Read,
    /// Transcripts carrying usage that this build could not read: the format
    /// moved.
    NotUnderstood,
}

/// Where each file was read up to.
#[derive(Debug, Clone, Copy)]
struct Mark {
    offset: u64,
}

/// Every reply in the last [`KEEP`], and where each file was left.
#[derive(Default)]
pub struct Ledger {
    marks: HashMap<PathBuf, Mark>,
    seen: HashSet<(String, String)>,
    spends: Vec<Spend>,
    understood: u64,
    misread: u64,
}

impl Ledger {
    /// Read whatever was written under `root` since the last time.
    pub fn read(&mut self, root: &Path, now: DateTime<Utc>) -> Found {
        let projects = root.join("projects");
        if !root.is_dir() {
            return Found::NoFolder;
        }
        let mut files = Vec::new();
        transcripts(&projects, &mut files);
        if files.is_empty() {
            return Found::Empty;
        }

        let oldest = SystemTime::from(now - KEEP);
        for file in files {
            let known = self.marks.contains_key(&file);
            // A file nobody has written to for longer than any window is
            // worth nothing here, and opening it is the expensive part.
            if !known {
                let touched = std::fs::metadata(&file).and_then(|m| m.modified());
                if touched.map(|at| at < oldest).unwrap_or(true) {
                    continue;
                }
            }
            self.read_file(&file);
        }
        self.forget_before(now - KEEP);

        if self.misread > 0 && self.understood == 0 {
            Found::NotUnderstood
        } else {
            Found::Read
        }
    }

    fn read_file(&mut self, file: &Path) {
        let Ok(mut handle) = std::fs::File::open(file) else { return };
        let Ok(length) = handle.metadata().map(|m| m.len()) else { return };
        let mark = self.marks.get(file).copied().unwrap_or(Mark { offset: 0 });
        // Shorter than where it was left: not the same file any more.
        let from = if length < mark.offset { 0 } else { mark.offset };
        if length == from || handle.seek(SeekFrom::Start(from)).is_err() {
            self.marks.insert(file.to_path_buf(), Mark { offset: from });
            return;
        }

        let mut text = Vec::with_capacity((length - from) as usize);
        if handle.take(length - from).read_to_end(&mut text).is_err() {
            return;
        }
        // Up to the last complete line: the one after it is still being
        // written, and half a line is not a reply.
        let Some(end) = text.iter().rposition(|b| *b == b'\n') else {
            self.marks.insert(file.to_path_buf(), Mark { offset: from });
            return;
        };
        for line in text[..end].split(|b| *b == b'\n') {
            self.take(line);
        }
        self.marks.insert(file.to_path_buf(), Mark { offset: from + end as u64 + 1 });
    }

    /// One line of a transcript.
    fn take(&mut self, line: &[u8]) {
        // Most lines are not replies, and looking for the word is far cheaper
        // than parsing a line to find it is not there.
        if !contains(line, b"\"usage\"") {
            return;
        }
        let Ok(parsed) = serde_json::from_slice::<Line>(line) else {
            self.misread += 1;
            return;
        };
        let Some(message) = parsed.message else { return };
        let Some(usage) = message.usage else { return };
        let (Some(at), Some(id)) = (parsed.timestamp, message.id) else {
            self.misread += 1;
            return;
        };
        let Ok(at) = DateTime::parse_from_rfc3339(&at) else {
            self.misread += 1;
            return;
        };
        self.understood += 1;
        if !self.seen.insert((id, parsed.request_id.unwrap_or_default())) {
            return;
        }
        let tokens = Tokens {
            input: usage.input_tokens.unwrap_or(0),
            output: usage.output_tokens.unwrap_or(0),
            cache_write: usage.cache_creation_input_tokens.unwrap_or(0),
            cache_read: usage.cache_read_input_tokens.unwrap_or(0),
        };
        if tokens == Tokens::default() {
            return;
        }
        self.spends.push(Spend { at: at.with_timezone(&Utc), tokens });
    }

    fn forget_before(&mut self, cut: DateTime<Utc>) {
        self.spends.retain(|spend| spend.at >= cut);
        self.spends.sort_by_key(|spend| spend.at);
    }

    /// The replies in the last [`KEEP`], oldest first.
    pub fn spends(&self) -> &[Spend] {
        &self.spends
    }
}

/// Everything spent in `[from, to)`.
pub fn between(spends: &[Spend], from: DateTime<Utc>, to: DateTime<Utc>) -> Tokens {
    let mut total = Tokens::default();
    for spend in spends.iter().filter(|s| s.at >= from && s.at < to) {
        total.add(&spend.tokens);
    }
    total
}

/// The five-hour session open at `now`, if there is one: when it started.
///
/// A session opens with the first message after the previous one closed,
/// and closes five hours after it opened. That is how the five-hour limit is
/// described, and it is reconstructed here from the replies alone — an
/// approximation, because the clock that really counts is on the server.
pub fn session_open_at(spends: &[Spend], now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let length = Duration::hours(5);
    let mut open: Option<DateTime<Utc>> = None;
    for spend in spends.iter().filter(|s| s.at <= now) {
        match open {
            Some(start) if spend.at < start + length => {}
            _ => open = Some(spend.at),
        }
    }
    open.filter(|start| now < *start + length)
}

/// Every `.jsonl` under `dir`, at any depth — subagents are a level down.
fn transcripts(dir: &Path, into: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => transcripts(&path, into),
            Ok(kind) if kind.is_file() && path.extension().is_some_and(|e| e == "jsonl") => {
                into.push(path)
            }
            _ => {}
        }
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|window| window == needle)
}

/// Where Claude Code keeps its files: `$CLAUDE_CONFIG_DIR`, as Claude Code
/// itself reads it, and `~/.claude` otherwise.
pub fn claude_dir() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".claude")))
}

/// The four fields of a line that are read. Everything else in it is
/// skipped by the parser without being kept.
#[derive(Deserialize)]
struct Line {
    timestamp: Option<String>,
    #[serde(rename = "requestId")]
    request_id: Option<String>,
    message: Option<Message>,
}

#[derive(Deserialize)]
struct Message {
    id: Option<String>,
    usage: Option<Usage>,
}

#[derive(Deserialize)]
struct Usage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::io::Write;

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir().join(format!("doca-usage-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(path.join("projects/-home-me-www-app")).unwrap();
            Self(path)
        }

        fn transcript(&self) -> PathBuf {
            self.0.join("projects/-home-me-www-app/session.jsonl")
        }

        fn append(&self, file: &Path, text: &str) {
            if let Some(parent) = file.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            let mut handle = std::fs::OpenOptions::new().create(true).append(true).open(file).unwrap();
            handle.write_all(text.as_bytes()).unwrap();
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn reply(id: &str, at: &str, input: u64, output: u64) -> String {
        format!(
            "{{\"type\":\"assistant\",\"timestamp\":\"{at}\",\"requestId\":\"req_{id}\",\"message\":{{\"id\":\"msg_{id}\",\"content\":[{{\"type\":\"text\",\"text\":\"a private answer\"}}],\"usage\":{{\"input_tokens\":{input},\"output_tokens\":{output},\"cache_creation_input_tokens\":100,\"cache_read_input_tokens\":5000}}}}}}\n"
        )
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 10, 17, 21, 0, 0).unwrap()
    }

    fn spent(ledger: &Ledger) -> u64 {
        between(ledger.spends(), now() - KEEP, now() + Duration::hours(1)).spent()
    }

    #[test]
    fn a_reply_written_once_per_block_is_counted_once() {
        let scratch = Scratch::new("once");
        let file = scratch.transcript();
        // The same reply three times, as Claude Code writes one per block.
        for _ in 0..3 {
            scratch.append(&file, &reply("a", "2026-10-17T20:00:00Z", 10, 20));
        }
        scratch.append(&file, &reply("b", "2026-10-17T20:05:00Z", 1, 2));
        let mut ledger = Ledger::default();
        assert_eq!(ledger.read(&scratch.0, now()), Found::Read);
        assert_eq!(spent(&ledger), (10 + 20 + 100) + (1 + 2 + 100));
        assert_eq!(ledger.spends().len(), 2);
    }

    #[test]
    fn what_was_read_is_not_read_again_and_half_a_line_waits() {
        let scratch = Scratch::new("tail");
        let file = scratch.transcript();
        scratch.append(&file, &reply("a", "2026-10-17T20:00:00Z", 10, 20));
        let mut ledger = Ledger::default();
        ledger.read(&scratch.0, now());
        assert_eq!(ledger.spends().len(), 1);

        // Half of the next reply, as a write caught in the middle.
        let next = reply("b", "2026-10-17T20:10:00Z", 1, 1);
        let (first, rest) = next.split_at(next.len() / 2);
        scratch.append(&file, first);
        ledger.read(&scratch.0, now());
        assert_eq!(ledger.spends().len(), 1, "half a line was taken for a reply");

        scratch.append(&file, rest);
        ledger.read(&scratch.0, now());
        assert_eq!(ledger.spends().len(), 2, "the line, finished, was not read");
        ledger.read(&scratch.0, now());
        assert_eq!(ledger.spends().len(), 2, "a second read of nothing new counted again");
    }

    #[test]
    fn a_file_that_shrank_is_read_again_from_the_start() {
        let scratch = Scratch::new("shrank");
        let file = scratch.transcript();
        scratch.append(&file, &reply("a", "2026-10-17T20:00:00Z", 10, 20));
        scratch.append(&file, &reply("b", "2026-10-17T20:01:00Z", 10, 20));
        let mut ledger = Ledger::default();
        ledger.read(&scratch.0, now());

        std::fs::write(&file, reply("c", "2026-10-17T20:30:00Z", 7, 7)).unwrap();
        ledger.read(&scratch.0, now());
        assert_eq!(ledger.spends().len(), 3);
    }

    #[test]
    fn subagents_count_and_lines_that_are_not_replies_do_not() {
        let scratch = Scratch::new("subagents");
        let file = scratch.transcript();
        scratch.append(&file, "{\"type\":\"user\",\"message\":{\"content\":\"hello\"}}\n");
        scratch.append(&file, &reply("a", "2026-10-17T20:00:00Z", 10, 20));
        let sub = scratch.0.join("projects/-home-me-www-app/uuid/subagents/agent-1.jsonl");
        scratch.append(&sub, &reply("s", "2026-10-17T20:02:00Z", 3, 3));
        let mut ledger = Ledger::default();
        ledger.read(&scratch.0, now());
        assert_eq!(ledger.spends().len(), 2);
    }

    #[test]
    fn nothing_there_and_a_format_that_moved_are_told_apart() {
        let mut ledger = Ledger::default();
        let nowhere = std::env::temp_dir().join(format!("doca-usage-{}-nowhere", std::process::id()));
        assert_eq!(ledger.read(&nowhere, now()), Found::NoFolder);

        let scratch = Scratch::new("empty");
        assert_eq!(Ledger::default().read(&scratch.0, now()), Found::Empty);

        // A usage that is no longer where it was: the word is there, the
        // shape is not.
        scratch.append(
            &scratch.transcript(),
            "{\"timestamp\":\"2026-10-17T20:00:00Z\",\"message\":{\"usage\":\"moved\"}}\n",
        );
        assert_eq!(Ledger::default().read(&scratch.0, now()), Found::NotUnderstood);
    }

    #[test]
    fn replies_older_than_any_window_are_forgotten() {
        let scratch = Scratch::new("old");
        let file = scratch.transcript();
        scratch.append(&file, &reply("old", "2026-10-01T20:00:00Z", 10, 20));
        scratch.append(&file, &reply("new", "2026-10-17T20:00:00Z", 1, 1));
        let mut ledger = Ledger::default();
        ledger.read(&scratch.0, now());
        assert_eq!(ledger.spends().len(), 1);
    }

    #[test]
    fn a_session_opens_on_the_first_message_after_the_last_one_closed() {
        let at = |h: u32, m: u32| Utc.with_ymd_and_hms(2026, 10, 17, h, m, 0).unwrap();
        let spend = |h: u32, m: u32| Spend { at: at(h, m), tokens: Tokens { input: 1, ..Tokens::default() } };
        let spends = vec![spend(9, 10), spend(11, 0), spend(14, 30), spend(15, 0)];

        // 09:10 opened one that ran to 14:10; 14:30 opened the next.
        assert_eq!(session_open_at(&spends, at(12, 0)), Some(at(9, 10)));
        assert_eq!(session_open_at(&spends, at(14, 20)), None, "closed, and nothing since");
        assert_eq!(session_open_at(&spends, at(16, 0)), Some(at(14, 30)));
        assert_eq!(session_open_at(&spends, at(19, 31)), None);
    }

    /// The issue asks for this measured on a real `~/.claude` with months
    /// in it, first read and the reads after. Counts and times only.
    ///
    /// ```text
    /// cargo test --release -p docad -- --ignored reads_this_machines_claude_code --nocapture
    /// ```
    #[test]
    #[ignore = "reads this user's own Claude Code folder"]
    fn reads_this_machines_claude_code() {
        let root = claude_dir().expect("a home");
        let mut ledger = Ledger::default();
        let started = std::time::Instant::now();
        let found = ledger.read(&root, Utc::now());
        let first = started.elapsed();
        let started = std::time::Instant::now();
        ledger.read(&root, Utc::now());
        let again = started.elapsed();
        let week = between(ledger.spends(), Utc::now() - Duration::days(7), Utc::now());
        println!(
            "{found:?}: {} files opened, {} replies kept, first read {first:?}, next {again:?}; \
             week: {} spent ({} cache read back not counted), {} lines understood, {} not",
            ledger.marks.len(),
            ledger.spends().len(),
            week.spent(),
            week.cache_read,
            ledger.understood,
            ledger.misread,
        );
        assert_eq!(found, Found::Read);
    }

    #[test]
    fn the_cache_read_back_is_kept_but_not_counted_as_spent() {
        let tokens = Tokens { input: 1, output: 2, cache_write: 3, cache_read: 1000 };
        assert_eq!(tokens.spent(), 6);
    }
}
