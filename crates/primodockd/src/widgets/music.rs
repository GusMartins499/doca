use std::time::Duration;

use primodock_ipc::{WidgetState, NO_PROGRESS};

use super::Widget;

const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NowPlaying {
    pub title: String,
    pub artist: String,
    pub player: String,
    pub playing: bool,
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

pub fn state_from(now_playing: Option<&NowPlaying>) -> WidgetState {
    match now_playing {
        Some(track) => WidgetState {
            id: "music".to_string(),
            label: if track.title.is_empty() {
                "unknown".to_string()
            } else {
                track.title.clone()
            },
            detail: if track.artist.is_empty() {
                track.player.clone()
            } else {
                track.artist.clone()
            },
            progress: NO_PROGRESS,
            active: track.playing,
        },
        None => WidgetState {
            id: "music".to_string(),
            label: "—".to_string(),
            detail: "nothing playing".to_string(),
            progress: NO_PROGRESS,
            active: false,
        },
    }
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

        Some(NowPlaying {
            title,
            artist,
            player: player_name(bus_name),
            playing: status == "Playing",
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
        assert!(!is_player("dev.oprimo.PrimoDock"));
    }

    #[test]
    fn a_playing_track_makes_the_tile_active() {
        let track = NowPlaying {
            title: "Garota de Ipanema".to_string(),
            artist: "João e Astrud".to_string(),
            player: "spotify".to_string(),
            playing: true,
        };

        let state = state_from(Some(&track));

        assert_eq!(state.label, "Garota de Ipanema");
        assert_eq!(state.detail, "João e Astrud");
        assert!(state.active);
    }

    #[test]
    fn a_paused_track_is_still_shown_but_not_active() {
        let track = NowPlaying {
            title: "Garota de Ipanema".to_string(),
            artist: "João e Astrud".to_string(),
            player: "spotify".to_string(),
            playing: false,
        };

        let state = state_from(Some(&track));

        assert_eq!(state.label, "Garota de Ipanema");
        assert!(!state.active);
    }

    #[test]
    fn no_player_at_all_still_renders_a_tile() {
        let state = state_from(None);

        assert_eq!(state.detail, "nothing playing");
        assert!(!state.active);
    }

    #[test]
    fn a_track_with_no_title_does_not_render_an_empty_label() {
        let track = NowPlaying {
            title: String::new(),
            artist: "Some Artist".to_string(),
            player: "vlc".to_string(),
            playing: true,
        };

        assert_eq!(state_from(Some(&track)).label, "unknown");
    }

    #[test]
    fn a_stream_with_no_artist_falls_back_to_naming_the_player() {
        let stream = NowPlaying {
            title: "Some radio stream".to_string(),
            artist: String::new(),
            player: "brave".to_string(),
            playing: true,
        };

        assert_eq!(state_from(Some(&stream)).detail, "brave");
    }
}
