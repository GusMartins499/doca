use std::time::Duration;

use doca_ipc::{Body, WidgetState};

use super::cover::Covers;
use super::Widget;

const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub player: String,
    pub playing: bool,
    /// The cover as a path on this machine, empty when there is none to be
    /// had — or none *yet*, while a remote one is on its way.
    pub art: String,
    /// The cover as the player named it, `file://` or `http(s)://`. Kept
    /// apart from `art` because turning one into the other may mean a
    /// download, and that is [`Covers`]' to do, not the bus query's.
    pub art_url: String,
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
    covers: Covers,
}

impl Music {
    pub fn new() -> Self {
        Self {
            connection: zbus::blocking::Connection::session().ok(),
            covers: Covers::new(),
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

        let art_url = metadata
            .get("mpris:artUrl")
            .and_then(|value| String::try_from(value.clone()).ok())
            .unwrap_or_default();

        Some(NowPlaying {
            title,
            artist,
            player: player_name(bus_name),
            playing: status == "Playing",
            art: String::new(),
            art_url,
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

    /// Which player the tile and the controls follow, when more than one is
    /// on the bus.
    ///
    /// **The first one that is playing wins, and otherwise the first that has
    /// a track at all.** Written down because the issue asked for it to be,
    /// and because "first" here means first in the order the bus lists names,
    /// which is not an order anybody chose.
    ///
    /// The rule is not arbitrary even if the order is. Two players *playing*
    /// at once is a state nobody wants and nobody can read anyway — whichever
    /// the tile picked, the other is still making noise — so the tile is
    /// worth no cleverness there. One playing and three paused is the case
    /// that actually happens, a browser tab left open behind the music, and
    /// for that "the one that is playing" is exactly right.
    ///
    /// What it is *not* is sticky: a player that stops hands the tile to the
    /// next one that is going rather than keeping it. That is the behaviour
    /// to revisit if this ever feels wrong, and it would want remembering
    /// which player the user last touched, which is state this widget does
    /// not have.
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

    /// Two seconds, except while the playing track's cover is downloading.
    ///
    /// The hub asks for the interval after each poll, so this is how a cover
    /// that lands half a second into a track is on the tile a quarter of a
    /// second later rather than up to two: the tile is announced with the
    /// track and without the cover, and again — a fresh `WidgetChanged` —
    /// when the cover is there.
    fn interval(&self) -> Duration {
        if self.covers.pending() {
            Duration::from_millis(250)
        } else {
            Duration::from_secs(2)
        }
    }

    fn poll(&mut self) -> WidgetState {
        let mut track = self.preferred_player().map(|(_, track)| track);
        // Asked every poll, with nothing playing too, so that a cover still
        // downloading for a track that has gone stops counting as wanted.
        let art_url = track.as_ref().map(|track| track.art_url.as_str()).unwrap_or("");
        let art = self.covers.resolve(art_url);
        if let Some(track) = track.as_mut() {
            track.art = art;
        }
        state_from(track.as_ref())
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
            art_url: String::new(),
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
}
