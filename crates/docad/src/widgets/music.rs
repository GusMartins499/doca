use std::time::Duration;

use doca_ipc::{Body, WidgetState};

use super::Widget;

const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub player: String,
    pub playing: bool,
    /// The cover as a path on this machine, empty when there is none to be
    /// had. See [`cover_path`] for what counts as having one.
    pub art: String,
}

/// The cover file an `mpris:artUrl` points at, if it points at one here.
///
/// Players spell this two ways. A `file://` URL is a picture already on this
/// disk — most local players write one into a cache of their own — and that
/// is the one taken. An `http(s)://` URL is a picture somewhere else, which
/// Spotify in particular returns, and fetching it means a network client in
/// the daemon and a cache with a ceiling: its own slice, with its own
/// dependency to argue about.
///
/// Anything else, or a file that is not there, is no cover. A path that does
/// not exist is worse than none: the bar would ask for it every frame and get
/// nothing, every track, for ever.
pub fn cover_path(art_url: &str) -> String {
    let Some(path) = art_url.strip_prefix("file://") else {
        return String::new();
    };
    // Percent-encoding, because a track called "Sign o' the Times" arrives as
    // `Sign%20o%27%20the%20Times` and a file of that name does not exist.
    let path = unescaped(path);
    if std::path::Path::new(&path).is_file() {
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

pub fn player_name(bus_name: &str) -> String {
    bus_name
        .strip_prefix(MPRIS_PREFIX)
        .unwrap_or(bus_name)
        .split('.')
        .next()
        .unwrap_or(bus_name)
        .to_string()
}

pub fn is_player(bus_name: &str) -> bool {
    bus_name.starts_with(MPRIS_PREFIX)
}

/// What the bar is told, which is what MPRIS said and nothing worked out
/// from it.
///
/// The track and the cover, as they are. Whether a missing title reads as
/// "unknown" or as the player's name, and what a tile with nothing playing
/// shows, is the bar's to decide — it is the one that knows how much room
/// there is to say it in.
pub fn state_from(now_playing: Option<&NowPlaying>) -> WidgetState {
    let track = now_playing.cloned().unwrap_or_default();
    WidgetState::new(
        "music",
        Body::Music(doca_ipc::Music {
            title: track.title,
            artist: track.artist,
            player: track.player,
            playing: track.playing,
            art: track.art,
        }),
    )
}

pub struct Music {
    connection: Option<zbus::blocking::Connection>,
}

impl Music {
    pub fn new() -> Self {
        Self {
            connection: zbus::blocking::Connection::session().ok(),
        }
    }

    fn players(&self) -> Vec<String> {
        let Some(connection) = &self.connection else {
            return Vec::new();
        };
        let Ok(proxy) = zbus::blocking::fdo::DBusProxy::new(connection) else {
            return Vec::new();
        };
        let Ok(names) = proxy.list_names() else {
            return Vec::new();
        };
        names
            .into_iter()
            .map(|name| name.to_string())
            .filter(|name| is_player(name))
            .collect()
    }

    fn now_playing(&self, bus_name: &str) -> Option<NowPlaying> {
        let connection = self.connection.as_ref()?;
        let proxy = zbus::blocking::Proxy::new(
            connection,
            bus_name.to_string(),
            "/org/mpris/MediaPlayer2",
            "org.mpris.MediaPlayer2.Player",
        )
        .ok()?;

        let status: String = proxy.get_property("PlaybackStatus").ok()?;
        let metadata: std::collections::HashMap<String, zbus::zvariant::OwnedValue> =
            proxy.get_property("Metadata").ok()?;

        let title = metadata
            .get("xesam:title")
            .and_then(|value| String::try_from(value.clone()).ok())
            .unwrap_or_default();
        let artist = metadata
            .get("xesam:artist")
            .and_then(|value| Vec::<String>::try_from(value.clone()).ok())
            .unwrap_or_default()
            .join(", ");

        if title.is_empty() && artist.is_empty() {
            return None;
        }

        let art = metadata
            .get("mpris:artUrl")
            .and_then(|value| String::try_from(value.clone()).ok())
            .map(|url| cover_path(&url))
            .unwrap_or_default();

        Some(NowPlaying {
            title,
            artist,
            player: player_name(bus_name),
            playing: status == "Playing",
            art,
        })
    }

    fn control(&self, bus_name: &str, method: &str) {
        let Some(connection) = self.connection.as_ref() else {
            return;
        };
        let Ok(proxy) = zbus::blocking::Proxy::new(
            connection,
            bus_name.to_string(),
            "/org/mpris/MediaPlayer2",
            "org.mpris.MediaPlayer2.Player",
        ) else {
            return;
        };
        let _: Result<(), _> = proxy.call(method, &());
    }

    fn preferred_player(&self) -> Option<(String, NowPlaying)> {
        let players = self.players();
        let mut fallback = None;
        for bus_name in players {
            let Some(track) = self.now_playing(&bus_name) else {
                continue;
            };
            if track.playing {
                return Some((bus_name, track));
            }
            fallback.get_or_insert((bus_name, track));
        }
        fallback
    }
}

impl Widget for Music {
    fn actions(&self) -> &'static [&'static str] {
        &["toggle", "next", "previous"]
    }

    fn id(&self) -> &str {
        "music"
    }

    fn interval(&self) -> Duration {
        Duration::from_secs(2)
    }

    fn poll(&mut self) -> WidgetState {
        state_from(self.preferred_player().as_ref().map(|(_, track)| track))
    }

    fn invoke(&mut self, action: &str) {
        let Some((bus_name, _)) = self.preferred_player() else {
            return;
        };
        let method = match action {
            "toggle" => "PlayPause",
            "next" => "Next",
            "previous" => "Previous",
            unknown => {
                tracing::warn!("music ignoring unknown action {unknown}");
                return;
            }
        };
        self.control(&bus_name, method);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_mpris_bus_name_yields_the_player_it_belongs_to() {
        assert_eq!(player_name("org.mpris.MediaPlayer2.spotify"), "spotify");
        assert_eq!(
            player_name("org.mpris.MediaPlayer2.brave.instance3786"),
            "brave"
        );
        assert_eq!(player_name("org.mpris.MediaPlayer2.vlc"), "vlc");
    }

    #[test]
    fn only_mpris_names_are_treated_as_players() {
        assert!(is_player("org.mpris.MediaPlayer2.spotify"));
        assert!(!is_player("org.freedesktop.DBus"));
        assert!(!is_player("io.github.gusmartins499.Doca"));
    }

    fn track(title: &str, artist: &str, player: &str, playing: bool) -> NowPlaying {
        NowPlaying {
            title: title.to_string(),
            artist: artist.to_string(),
            player: player.to_string(),
            playing,
            art: String::new(),
        }
    }

    fn body_of(state: &WidgetState) -> doca_ipc::Music {
        match state.body().expect("the body reads back") {
            Body::Music(music) => music,
            other => panic!("music sent {other:?}"),
        }
    }

    /// What the daemon sends is what MPRIS said, and nothing worked out from
    /// it.
    ///
    /// These used to assert the words — "unknown" for a track with no title,
    /// the player's name for a stream with no artist. Those are the bar's to
    /// choose now, because the bar is the one that knows how much room there
    /// is to say them in; what is held here is that the daemon passes the
    /// track through without inventing or dropping anything.
    #[test]
    fn what_is_sent_is_what_the_player_said() {
        let playing = track("Garota de Ipanema", "João e Astrud", "spotify", true);

        let music = body_of(&state_from(Some(&playing)));

        assert_eq!(music.title, "Garota de Ipanema");
        assert_eq!(music.artist, "João e Astrud");
        assert_eq!(music.player, "spotify");
        assert!(music.playing);
    }

    #[test]
    fn a_paused_track_is_still_sent_but_not_as_playing() {
        let paused = track("Garota de Ipanema", "João e Astrud", "spotify", false);

        let music = body_of(&state_from(Some(&paused)));

        assert_eq!(music.title, "Garota de Ipanema");
        assert!(!music.playing);
    }

    /// An empty field stays empty rather than being filled in down here: a
    /// title the daemon guessed at is a title the bar cannot tell from a real
    /// one.
    #[test]
    fn a_track_missing_a_field_arrives_missing_it() {
        let untitled = track("", "Some Artist", "vlc", true);
        let streaming = track("Some radio stream", "", "brave", true);

        assert_eq!(body_of(&state_from(Some(&untitled))).title, "");
        assert_eq!(body_of(&state_from(Some(&streaming))).artist, "");
        assert_eq!(body_of(&state_from(Some(&streaming))).player, "brave");
    }

    #[test]
    fn no_player_at_all_still_renders_a_tile() {
        let music = body_of(&state_from(None));

        assert_eq!(music.title, "");
        assert_eq!(music.player, "");
        assert!(!music.playing);
    }

    /// A `file://` cover is one already on this disk, and the only kind this
    /// slice takes. Percent-encoding is undone, because a track called
    /// "Sign o' the Times" arrives with its apostrophe spelled `%27` and a
    /// file of that name does not exist.
    #[test]
    fn a_cover_already_on_this_disk_is_the_one_that_is_taken() {
        let dir = std::env::temp_dir().join(format!("doca-cover-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("Sign o' the Times.png");
        std::fs::write(&file, b"not really a png").unwrap();

        let url = format!("file://{}", file.display().to_string().replace(' ', "%20").replace('\'', "%27"));

        assert_eq!(cover_path(&url), file.display().to_string());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A path that is not there is worse than no cover: the bar would ask for
    /// it every frame, every track, for ever.
    #[test]
    fn a_cover_that_is_not_there_is_no_cover() {
        assert_eq!(cover_path("file:///nowhere/at/all.png"), "");
    }

    /// Remote art is somebody else's slice, and until it exists it has to
    /// read as no art rather than as a path nothing can open.
    #[test]
    fn a_cover_somewhere_else_is_not_a_path_on_this_machine() {
        assert_eq!(cover_path("https://i.scdn.co/image/abc123"), "");
        assert_eq!(cover_path(""), "");
        assert_eq!(cover_path("nonsense"), "");
    }
}
