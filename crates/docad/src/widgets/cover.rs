//! An album cover, as a path on this machine.
//!
//! The transport was decided in #29: the bar is handed the path of a file and
//! opens it, the way it already opens `DockItem.icon`. A `file://` cover is
//! that already. An `http(s)://` one — Spotify's — has to become it first,
//! which is the only reason this module has a network client and a cache.
//!
//! **Nothing here ever waits on the network where anybody can see it.** The
//! widget thread asks [`Covers::resolve`] for a path, which is a `stat` at
//! worst: a cover not on disk yet is answered with "none" at once, and a
//! thread of its own goes and gets it. The next poll finds the file. The
//! track changes on the tile the moment it changes; the cover follows.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

/// How much disk the cover cache may take before the least recently shown
/// covers go.
///
/// A constant rather than a setting: a 640px JPEG is about 100 KiB, so this
/// is a few hundred covers, which nobody will ever want to tune. If somebody
/// does, it belongs in `[widgets.music]` of the config.
pub const CEILING: u64 = 32 * 1024 * 1024;

/// The largest single cover worth keeping. Anything bigger is refused before
/// it is all read: a URL that answers with a video is not a cover.
pub const LARGEST: u64 = 4 * 1024 * 1024;

/// How long one fetch may take, from connecting to the last byte. A cover
/// that has not arrived by then is arriving for a track that is half over.
const WHOLE: Duration = Duration::from_secs(15);
const CONNECT: Duration = Duration::from_secs(5);

/// How long a URL that failed is left alone. Without it the widget, polling
/// every two seconds, would try a dead URL — and log it — every two seconds
/// for as long as the track plays.
const RETRY_AFTER: Duration = Duration::from_secs(10 * 60);

/// How many fetches may be running at once. Skipping through a playlist
/// starts one per track; past this many, the tracks skipped over simply do
/// not get theirs, and the one that stays playing gets its turn on the next
/// poll.
const IN_FLIGHT: usize = 4;

/// The cover file an `mpris:artUrl` points at, if it is one on this disk.
///
/// Players spell this two ways. A `file://` URL is a picture already here —
/// most local players write one into a cache of their own — and this is the
/// whole of the answer for it. An `http(s)://` URL is somewhere else, and is
/// [`Covers`]' to fetch; here it is no path at all.
///
/// A file that is not there is no cover either. A path that does not exist
/// is worse than none: the bar would ask for it every frame and get nothing,
/// every track, for ever.
pub fn cover_path(art_url: &str) -> String {
    let Some(path) = art_url.strip_prefix("file://") else {
        return String::new();
    };
    // Percent-encoding, because a track called "Sign o' the Times" arrives as
    // `Sign%20o%27%20the%20Times` and a file of that name does not exist.
    let path = unescaped(path);
    if Path::new(&path).is_file() {
        path
    } else {
        String::new()
    }
}

/// `%20` and friends, turned back into the bytes they stand for.
fn unescaped(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' && at + 2 < bytes.len() {
            let pair = std::str::from_utf8(&bytes[at + 1..at + 3]).ok();
            if let Some(byte) = pair.and_then(|pair| u8::from_str_radix(pair, 16).ok()) {
                out.push(byte);
                at += 3;
                continue;
            }
        }
        out.push(bytes[at]);
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn is_remote(art_url: &str) -> bool {
    art_url.starts_with("https://") || art_url.starts_with("http://")
}

/// Where fetched covers are kept: `$XDG_CACHE_HOME/doca/covers`, then
/// `~/.cache/doca/covers`.
///
/// A directory of their own inside `doca/` so that the eviction, which
/// deletes whatever is oldest in it, can only ever delete a cover.
pub fn cache_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        // The spec says a relative value is to be ignored, and a cache in
        // whatever directory the daemon was started from is a mess nobody
        // would think to look for.
        .filter(|base| base.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
    Some(base.join("doca/covers"))
}

/// The cache's name for a URL: FNV-1a over its bytes, in hex.
///
/// Spelled out rather than `DefaultHasher`, whose output the standard library
/// is free to change between releases — and a cache whose names change when
/// the compiler does is one that is downloaded again after every upgrade and
/// never cleans up the old copies until the ceiling does. Sixty-four bits is
/// plenty for a few hundred covers; a collision would show one album's cover
/// on another's track, not crash anything.
pub fn key(url: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in url.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Whether these bytes start the way a picture the bar can open starts.
///
/// The bytes rather than the `Content-Type`: a server that calls an error
/// page `image/jpeg` is not rare, and what matters is whether the bar can
/// draw it, which is decided by what is in the file.
pub fn is_image(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xff, 0xd8, 0xff])
        || bytes.starts_with(b"\x89PNG\r\n\x1a\n")
        || bytes.starts_with(b"GIF87a")
        || bytes.starts_with(b"GIF89a")
        || (bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP")
        || bytes.starts_with(b"BM")
}

/// Covers on disk, at most [`CEILING`] of them, least recently shown first
/// out.
///
/// "Recently shown" is the file's mtime, which [`Cache::lookup`] moves
/// forward on every hit. That keeps the bookkeeping in the filesystem, where
/// it survives a restart, rather than in an index that would have to be
/// written and could disagree with what is actually there.
pub struct Cache {
    dir: PathBuf,
    ceiling: u64,
}

impl Cache {
    pub fn new(dir: PathBuf, ceiling: u64) -> Self {
        Self { dir, ceiling }
    }

    pub fn path_for(&self, url: &str) -> PathBuf {
        self.dir.join(key(url))
    }

    /// The cover for this URL, if it has been fetched before.
    pub fn lookup(&self, url: &str) -> Option<PathBuf> {
        let path = self.path_for(url);
        if !path.is_file() {
            return None;
        }
        // Failing to touch it costs only its place in the queue.
        let _ = std::fs::File::options()
            .write(true)
            .open(&path)
            .and_then(|file| file.set_modified(SystemTime::now()));
        Some(path)
    }

    /// Keep these bytes as the cover for this URL, and make room for them.
    pub fn store(&self, url: &str, bytes: &[u8]) -> anyhow::Result<PathBuf> {
        let path = self.path_for(url);
        // Whole or not at all: the bar may open it the instant it appears.
        crate::atomic::write(&path, bytes)?;
        self.evict(&path);
        Ok(path)
    }

    /// Delete the oldest covers until what is left fits under the ceiling,
    /// never the one just written.
    fn evict(&self, keep: &Path) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        let mut covers: Vec<(SystemTime, u64, PathBuf)> = entries
            .filter_map(|entry| entry.ok())
            // A dot is another write's scratch file, mid-rename.
            .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
            .filter_map(|entry| {
                let meta = entry.metadata().ok()?;
                meta.is_file().then(|| {
                    let at = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                    (at, meta.len(), entry.path())
                })
            })
            .collect();
        let mut total: u64 = covers.iter().map(|(_, len, _)| len).sum();
        covers.sort();
        for (_, len, path) in covers {
            if total <= self.ceiling {
                break;
            }
            if path == keep {
                continue;
            }
            if std::fs::remove_file(&path).is_ok() {
                total -= len;
            }
        }
    }
}

/// Getting a cover's bytes from wherever its URL says.
///
/// A trait so the tests can stand in for the network; [`Http`] is the only
/// one the daemon uses.
pub trait Fetch: Send + Sync {
    fn fetch(&self, url: &str) -> Result<Vec<u8>, String>;
}

/// The network, through `ureq`.
///
/// `ureq` because it is blocking, and blocking is what fits: the widgets are
/// plain threads polling on a timer, and a fetch is a thread of its own that
/// waits on a socket and ends. The daemon does have tokio, but for the bus;
/// pulling `reqwest` in would bring hyper and a second HTTP stack to
/// download one JPEG per track, and calling into the runtime from the widget
/// thread would tie the two halves of the daemon together for no gain.
pub struct Http {
    agent: ureq::Agent,
    largest: u64,
}

impl Http {
    pub fn new(whole: Duration, largest: u64) -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(whole))
            .timeout_connect(Some(CONNECT.min(whole)))
            .max_redirects(4)
            .build();
        Self {
            agent: ureq::Agent::new_with_config(config),
            largest,
        }
    }
}

impl Fetch for Http {
    fn fetch(&self, url: &str) -> Result<Vec<u8>, String> {
        let mut response = self.agent.get(url).call().map_err(|e| e.to_string())?;
        // The limit is checked as the bytes arrive, so a URL that answers
        // with a gigabyte costs `largest` of it and no more.
        response
            .body_mut()
            .with_config()
            .limit(self.largest)
            .read_to_vec()
            .map_err(|e| e.to_string())
    }
}

/// What a fetch thread reports back: which URL it was for, and whether its
/// cover is now in the cache.
type Outcome = (String, Result<(), String>);

/// The widget's side of the covers: a path now, or none now and a fetch
/// started.
pub struct Covers {
    cache: Option<Arc<Cache>>,
    fetch: Arc<dyn Fetch>,
    /// The URL the last poll asked about — the track that is playing.
    wanted: String,
    /// The last remote cover found, so that two seconds of the same track is
    /// one `stat` and not a rewrite of the file's mtime every poll.
    shown: Option<(String, String)>,
    in_flight: HashSet<String>,
    failed: HashMap<String, Instant>,
    done_tx: mpsc::Sender<Outcome>,
    done_rx: mpsc::Receiver<Outcome>,
}

impl Covers {
    pub fn new() -> Self {
        Self::with(
            cache_dir().map(|dir| Cache::new(dir, CEILING)),
            Arc::new(Http::new(WHOLE, LARGEST)),
        )
    }

    pub fn with(cache: Option<Cache>, fetch: Arc<dyn Fetch>) -> Self {
        let (done_tx, done_rx) = mpsc::channel();
        Self {
            cache: cache.map(Arc::new),
            fetch,
            wanted: String::new(),
            shown: None,
            in_flight: HashSet::new(),
            failed: HashMap::new(),
            done_tx,
            done_rx,
        }
    }

    /// The cover for this `mpris:artUrl` as a path to hand the bar, or an
    /// empty string for the tile's resting state.
    ///
    /// Never waits. A remote cover not yet on disk is answered with nothing,
    /// and fetched behind the tile's back; it is the next call with the same
    /// URL that finds it.
    ///
    /// **A fetch that finishes for a track no longer playing shows nothing.**
    /// Not because its result is thrown away — the file is kept, the track
    /// may come round again — but because nothing is ever shown that was not
    /// looked up by the URL being asked about *now*. A download cannot hand a
    /// path to the tile; it can only put a file where the right question will
    /// find it.
    pub fn resolve(&mut self, art_url: &str) -> String {
        self.collect();
        self.wanted = art_url.to_string();

        // Already on this disk, in the player's own cache: copying it into
        // ours would be a cache of a cache.
        if !is_remote(art_url) {
            return cover_path(art_url);
        }
        let Some(cache) = &self.cache else {
            return String::new();
        };
        if let Some((url, path)) = &self.shown {
            if url == art_url && Path::new(path).is_file() {
                return path.clone();
            }
        }
        if let Some(path) = cache.lookup(art_url) {
            let path = path.display().to_string();
            self.shown = Some((art_url.to_string(), path.clone()));
            return path;
        }
        self.start(art_url);
        String::new()
    }

    /// Whether the cover being asked about is on its way, which is when the
    /// widget should look again sooner than it otherwise would.
    pub fn pending(&self) -> bool {
        self.in_flight.contains(&self.wanted)
    }

    /// Take in whatever the fetch threads have finished since last time.
    fn collect(&mut self) {
        while let Ok((url, outcome)) = self.done_rx.try_recv() {
            self.in_flight.remove(&url);
            if let Err(reason) = outcome {
                // Once per URL per `RETRY_AFTER`, because that is how often
                // it is tried: a dead link does not fill the journal.
                tracing::warn!("no cover from {url}: {reason}");
                self.failed.insert(url, Instant::now());
            }
        }
    }

    fn start(&mut self, url: &str) {
        self.failed.retain(|_, at| at.elapsed() < RETRY_AFTER);
        if self.in_flight.contains(url)
            || self.failed.contains_key(url)
            || self.in_flight.len() >= IN_FLIGHT
        {
            return;
        }
        let Some(cache) = self.cache.clone() else {
            return;
        };
        let fetch = Arc::clone(&self.fetch);
        let done = self.done_tx.clone();
        let owned = url.to_string();
        let spawned = std::thread::Builder::new()
            .name("cover".into())
            .spawn(move || {
                let outcome = fetch.fetch(&owned).and_then(|bytes| {
                    if !is_image(&bytes) {
                        return Err(format!("{} bytes that are not a picture", bytes.len()));
                    }
                    cache.store(&owned, &bytes).map(|_| ()).map_err(|e| format!("{e:#}"))
                });
                let _ = done.send((owned, outcome));
            });
        match spawned {
            Ok(_) => {
                self.in_flight.insert(url.to_string());
            }
            Err(e) => tracing::warn!("cannot start fetching a cover: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    /// A directory of this test's own, cleaned up after it.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let path =
                std::env::temp_dir().join(format!("doca-cover-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn files(&self) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(&self.0)
                .map(|entries| {
                    entries
                        .filter_map(|entry| entry.ok())
                        .map(|entry| entry.file_name().to_string_lossy().into_owned())
                        .collect()
                })
                .unwrap_or_default();
            names.sort();
            names
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The smallest thing that passes for a JPEG, with a tag to tell covers
    /// apart by.
    fn jpeg(tag: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0xff, 0xd8, 0xff, 0xe0];
        bytes.extend_from_slice(tag);
        bytes
    }

    /// The same bytes for every URL, counting how often it was asked.
    struct Canned {
        bytes: Vec<u8>,
        calls: AtomicUsize,
    }

    impl Canned {
        fn new(bytes: Vec<u8>) -> Arc<Self> {
            Arc::new(Self { bytes, calls: AtomicUsize::new(0) })
        }
    }

    impl Fetch for Canned {
        fn fetch(&self, _url: &str) -> Result<Vec<u8>, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.bytes.clone())
        }
    }

    /// A network that answers each URL only when the test says so.
    struct Gated(Mutex<HashMap<String, mpsc::Receiver<Vec<u8>>>>);

    impl Fetch for Gated {
        fn fetch(&self, url: &str) -> Result<Vec<u8>, String> {
            let gate = self.0.lock().unwrap().remove(url).ok_or("no gate")?;
            gate.recv().map_err(|e| e.to_string())
        }
    }

    /// Ask until there is an answer or nothing is on its way.
    fn settled(covers: &mut Covers, url: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let path = covers.resolve(url);
            if !path.is_empty() || !covers.pending() || Instant::now() > deadline {
                return path;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn a_url_has_the_same_name_in_the_cache_every_time() {
        let url = "https://i.scdn.co/image/ab67616d0000b273";

        assert_eq!(key(url), key(url));
        // Pinned, because the point is that it survives a new compiler: a
        // name that drifts is a cache downloaded again after every upgrade.
        assert_eq!(key(""), "cbf29ce484222325");
        assert_eq!(key("a"), "af63dc4c8601ec8c");
        assert_ne!(key(url), key("https://i.scdn.co/image/ab67616d0000b274"));
        assert!(key(url).chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn a_cover_fetched_once_is_not_fetched_again() {
        let scratch = Scratch::new("once");
        let url = "https://covers.example/one";
        let canned = Canned::new(jpeg(b"one"));

        let mut first = Covers::with(Some(Cache::new(scratch.0.clone(), CEILING)), canned.clone());
        assert_eq!(first.resolve(url), "", "the first ask must not wait for the network");
        let path = settled(&mut first, url);
        assert_eq!(std::fs::read(&path).unwrap(), jpeg(b"one"));

        // A daemon started again finds it on disk.
        let mut again = Covers::with(Some(Cache::new(scratch.0.clone(), CEILING)), canned.clone());
        assert_eq!(again.resolve(url), path);
        assert_eq!(canned.calls.load(Ordering::SeqCst), 1);
    }

    /// The ceiling holds, and what goes is what was shown longest ago — not
    /// what was fetched longest ago, which would throw out the album on
    /// repeat.
    #[test]
    fn the_cache_stays_under_its_ceiling_by_dropping_the_least_recently_shown() {
        let scratch = Scratch::new("ceiling");
        let cache = Cache::new(scratch.0.clone(), 100);
        let aged = |url: &str, ago: u64| {
            std::fs::File::options()
                .write(true)
                .open(cache.path_for(url))
                .unwrap()
                .set_modified(SystemTime::now() - Duration::from_secs(ago))
                .unwrap();
        };

        cache.store("old-but-shown", &[0u8; 40]).unwrap();
        aged("old-but-shown", 100);
        cache.store("newer", &[0u8; 40]).unwrap();
        aged("newer", 50);
        assert!(cache.lookup("old-but-shown").is_some());
        cache.store("newest", &[0u8; 40]).unwrap();

        assert!(cache.path_for("old-but-shown").is_file());
        assert!(!cache.path_for("newer").is_file(), "the least recently shown stays");
        assert!(cache.path_for("newest").is_file());
        let total: u64 = scratch
            .files()
            .iter()
            .map(|name| std::fs::metadata(scratch.0.join(name)).unwrap().len())
            .sum();
        assert!(total <= 100, "{total} bytes kept under a ceiling of 100");
    }

    /// A `file://` cover is in the player's own cache already, and a second
    /// copy in ours would be a cache of a cache.
    #[test]
    fn a_cover_already_on_disk_is_not_copied_into_the_cache() {
        let scratch = Scratch::new("local");
        let cache_dir = scratch.0.join("cache");
        let file = scratch.0.join("cover.png");
        std::fs::write(&file, b"\x89PNG\r\n\x1a\nlocal").unwrap();
        let canned = Canned::new(jpeg(b"remote"));
        let mut covers = Covers::with(Some(Cache::new(cache_dir.clone(), CEILING)), canned.clone());

        let url = format!("file://{}", file.display());

        assert_eq!(covers.resolve(&url), file.display().to_string());
        assert!(!covers.pending());
        assert_eq!(canned.calls.load(Ordering::SeqCst), 0);
        assert!(!cache_dir.exists() || std::fs::read_dir(&cache_dir).unwrap().next().is_none());
    }

    /// The track changes while the old cover is still downloading. The old
    /// one arriving must not land on the new track — not even for one poll.
    #[test]
    fn a_cover_arriving_for_the_previous_track_is_not_shown_on_this_one() {
        let scratch = Scratch::new("stale");
        let (old_tx, old_rx) = mpsc::channel();
        let (new_tx, new_rx) = mpsc::channel();
        let gates = HashMap::from([
            ("https://covers.example/old".to_string(), old_rx),
            ("https://covers.example/new".to_string(), new_rx),
        ]);
        let mut covers = Covers::with(
            Some(Cache::new(scratch.0.clone(), CEILING)),
            Arc::new(Gated(Mutex::new(gates))),
        );
        let old = "https://covers.example/old";
        let new = "https://covers.example/new";

        assert_eq!(covers.resolve(old), "");
        assert_eq!(covers.resolve(new), "", "the track changed; the tile rests");

        old_tx.send(jpeg(b"old")).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while covers.in_flight.contains(old) && Instant::now() < deadline {
            assert_eq!(covers.resolve(new), "", "the old cover showed on the new track");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!covers.in_flight.contains(old), "the old fetch never came back");
        assert_eq!(covers.resolve(new), "");

        new_tx.send(jpeg(b"new")).unwrap();
        let path = settled(&mut covers, new);
        assert_eq!(std::fs::read(&path).unwrap(), jpeg(b"new"));
    }

    /// An error page, a redirect to a login form: bytes that are not a
    /// picture stay out of the cache, and the URL is not asked again every
    /// poll.
    #[test]
    fn a_response_that_is_not_a_picture_leaves_the_tile_resting() {
        let scratch = Scratch::new("html");
        let canned = Canned::new(b"<!doctype html><p>nope".to_vec());
        let mut covers = Covers::with(Some(Cache::new(scratch.0.clone(), CEILING)), canned.clone());
        let url = "https://covers.example/broken";

        assert_eq!(settled(&mut covers, url), "");
        for _ in 0..5 {
            assert_eq!(covers.resolve(url), "");
        }

        assert_eq!(canned.calls.load(Ordering::SeqCst), 1);
        assert!(scratch.files().is_empty());
    }

    #[test]
    fn picture_formats_are_told_from_everything_else() {
        assert!(is_image(&jpeg(b"")));
        assert!(is_image(b"\x89PNG\r\n\x1a\n...."));
        assert!(is_image(b"GIF89a...."));
        assert!(is_image(b"RIFF\0\0\0\0WEBPVP8 "));
        assert!(!is_image(b"<html>"));
        assert!(!is_image(b""));
        assert!(!is_image(b"RIFF\0\0\0\0WAVEfmt "));
    }

    #[test]
    fn the_cache_lives_under_the_xdg_cache_directory() {
        // Read rather than set: the environment is shared with every other
        // test in the process.
        let dir = cache_dir();
        if let Some(dir) = dir {
            assert!(dir.ends_with("doca/covers"));
            assert!(dir.is_absolute());
        }
    }

    /// A local server that answers one request with whatever it is given,
    /// after however long it is told to wait.
    fn serve_once(response: Vec<u8>, delay: Duration) -> String {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut seen = Vec::new();
            let mut buf = [0u8; 1024];
            while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => seen.extend_from_slice(&buf[..n]),
                }
            }
            std::thread::sleep(delay);
            let _ = stream.write_all(&response);
        });
        format!("http://127.0.0.1:{port}/cover")
    }

    fn ok(body: &[u8]) -> Vec<u8> {
        let mut response =
            format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len())
                .into_bytes();
        response.extend_from_slice(body);
        response
    }

    #[test]
    fn a_cover_comes_back_over_http() {
        let url = serve_once(ok(&jpeg(b"wire")), Duration::ZERO);

        let bytes = Http::new(Duration::from_secs(5), LARGEST).fetch(&url).unwrap();

        assert_eq!(bytes, jpeg(b"wire"));
    }

    #[test]
    fn a_response_larger_than_a_cover_is_refused() {
        let url = serve_once(ok(&vec![0xffu8; 4096]), Duration::ZERO);

        assert!(Http::new(Duration::from_secs(5), 1024).fetch(&url).is_err());
    }

    #[test]
    fn a_server_that_never_answers_is_given_up_on() {
        let url = serve_once(ok(&jpeg(b"late")), Duration::from_secs(3));
        let started = Instant::now();

        assert!(Http::new(Duration::from_millis(300), LARGEST).fetch(&url).is_err());
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn a_missing_cover_is_an_error_rather_than_an_error_page() {
        let url = serve_once(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 4\r\nConnection: close\r\n\r\nnope".to_vec(),
            Duration::ZERO,
        );

        assert!(Http::new(Duration::from_secs(5), LARGEST).fetch(&url).is_err());
    }

    /// A `file://` cover is one already on this disk. Percent-encoding is
    /// undone, because a track called "Sign o' the Times" arrives with its
    /// apostrophe spelled `%27` and a file of that name does not exist.
    #[test]
    fn a_cover_already_on_this_disk_is_the_one_that_is_taken() {
        let scratch = Scratch::new("percent");
        let file = scratch.0.join("Sign o' the Times.png");
        std::fs::write(&file, b"not really a png").unwrap();

        let url = format!(
            "file://{}",
            file.display().to_string().replace(' ', "%20").replace('\'', "%27")
        );

        assert_eq!(cover_path(&url), file.display().to_string());
    }

    /// A path that is not there is worse than no cover: the bar would ask for
    /// it every frame, every track, for ever.
    #[test]
    fn a_cover_that_is_not_there_is_no_cover() {
        assert_eq!(cover_path("file:///nowhere/at/all.png"), "");
    }

    /// A remote URL is not a path on this machine until it has been fetched,
    /// and anything that is neither kind is nothing at all.
    #[test]
    fn a_cover_somewhere_else_is_not_a_path_on_this_machine() {
        assert_eq!(cover_path("https://i.scdn.co/image/abc123"), "");
        assert_eq!(cover_path(""), "");
        assert_eq!(cover_path("nonsense"), "");

        let mut covers = Covers::with(None, Canned::new(jpeg(b"")));
        assert_eq!(covers.resolve("nonsense"), "");
        assert!(!covers.pending());
    }
}
