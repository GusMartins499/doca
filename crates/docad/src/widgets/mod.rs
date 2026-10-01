pub mod battery;
pub mod clock;
pub mod countdown;
pub mod cpu;
pub mod note;
pub mod timer;
pub mod water;
pub mod music;
pub mod network;
pub mod pomodoro;
pub mod stopwatch;
pub mod time_progress;

use std::time::{Duration, Instant};

use anyhow::Result;
use doca_ipc::WidgetState;

use crate::config::WidgetSettings;

pub trait Widget: Send {
    fn id(&self) -> &str;
    fn interval(&self) -> Duration;
    fn poll(&mut self) -> WidgetState;
    fn invoke(&mut self, _action: &str) {}

    /// Take the settings as they now are, keeping whatever is being counted.
    ///
    /// The alternative — building the widget again from the new settings —
    /// would answer "the water goal is 10 now" by forgetting the six glasses
    /// already drunk and stopping a running timer. Most widgets take no
    /// settings at all and the default no-op is the whole of their answer.
    fn adopt(&mut self, _settings: &WidgetSettings) {}
}

const TICK: Duration = Duration::from_millis(250);

enum Request {
    List(async_channel::Sender<Vec<WidgetState>>),
    Invoke(String, String, async_channel::Sender<()>),
    /// Hold exactly these widgets, with exactly these settings.
    Settle(Vec<String>, WidgetSettings, async_channel::Sender<()>),
}

/// Where each wanted widget comes from: an index into what the hub already
/// holds, or `None` for one that has to be built.
///
/// Separated from the hub so it can be tested without a thread. The whole
/// point is the `Some` case: a widget that is still wanted is carried over
/// rather than rebuilt, so changing one dock's widget list does not reset the
/// stopwatch running in another.
pub fn reconcile(have: &[String], want: &[String]) -> Vec<Option<usize>> {
    let mut taken = vec![false; have.len()];
    want.iter()
        .map(|id| {
            let at = (0..have.len()).find(|at| !taken[*at] && &have[*at] == id)?;
            taken[at] = true;
            Some(at)
        })
        .collect()
}

#[derive(Clone)]
pub struct WidgetHandle {
    tx: async_channel::Sender<Request>,
}

pub fn spawn_hub(
    mut widgets: Vec<Box<dyn Widget>>,
) -> Result<(WidgetHandle, async_channel::Receiver<WidgetState>)> {
    let (tx, rx) = async_channel::unbounded::<Request>();
    let (changes_tx, changes_rx) = async_channel::unbounded::<WidgetState>();

    std::thread::Builder::new()
        .name("widgets".into())
        .spawn(move || {
            let mut last: Vec<Option<WidgetState>> = vec![None; widgets.len()];
            let mut due: Vec<Instant> = vec![Instant::now(); widgets.len()];

            loop {
                while let Ok(request) = rx.try_recv() {
                    match request {
                        Request::List(reply) => {
                            let states = widgets
                                .iter_mut()
                                .enumerate()
                                .map(|(index, widget)| match &last[index] {
                                    Some(state) => state.clone(),
                                    None => {
                                        let state = widget.poll();
                                        last[index] = Some(state.clone());
                                        state
                                    }
                                })
                                .collect();
                            let _ = reply.send_blocking(states);
                        }
                        Request::Invoke(id, action, reply) => {
                            if let Some(index) =
                                widgets.iter().position(|widget| widget.id() == id)
                            {
                                widgets[index].invoke(&action);
                                due[index] = Instant::now();
                            }
                            let _ = reply.send_blocking(());
                        }
                        Request::Settle(ids, settings, reply) => {
                            let held: Vec<String> =
                                widgets.iter().map(|widget| widget.id().to_string()).collect();
                            let plan = reconcile(&held, &ids);

                            let mut carried: Vec<Option<Box<dyn Widget>>> =
                                widgets.drain(..).map(Some).collect();
                            let mut was_last: Vec<Option<Option<WidgetState>>> =
                                last.drain(..).map(Some).collect();
                            let mut was_due: Vec<Option<Instant>> =
                                due.drain(..).map(Some).collect();

                            for (wanted, from) in ids.iter().zip(plan) {
                                match from {
                                    Some(at) => {
                                        let Some(mut widget) = carried[at].take() else {
                                            continue;
                                        };
                                        widget.adopt(&settings);
                                        widgets.push(widget);
                                        last.push(was_last[at].take().unwrap_or(None));
                                        due.push(
                                            was_due[at].take().unwrap_or_else(Instant::now),
                                        );
                                    }
                                    // Built one at a time rather than in one
                                    // `build` call whose results are taken in
                                    // order: `build` skips an id nobody wrote,
                                    // so any such pairing drifts by one the
                                    // first time an id is unknown — or just
                                    // carried over.
                                    None => match build(
                                        std::slice::from_ref(wanted),
                                        &settings,
                                    )
                                    .pop()
                                    {
                                        Some(widget) => {
                                            widgets.push(widget);
                                            last.push(None);
                                            due.push(Instant::now());
                                        }
                                        None => tracing::warn!(
                                            "nothing to build for widget {wanted}"
                                        ),
                                    },
                                }
                            }

                            // Anything that settled differently says so now
                            // rather than at its own next tick, which for a
                            // note is an hour away.
                            for (index, widget) in widgets.iter_mut().enumerate() {
                                let state = widget.poll();
                                due[index] = Instant::now() + widget.interval();
                                if last[index].as_ref() == Some(&state) {
                                    continue;
                                }
                                last[index] = Some(state.clone());
                                let _ = changes_tx.send_blocking(state);
                            }
                            let _ = reply.send_blocking(());
                        }
                    }
                }
                if rx.is_closed() {
                    break;
                }

                let now = Instant::now();
                for (index, widget) in widgets.iter_mut().enumerate() {
                    if now < due[index] {
                        continue;
                    }
                    due[index] = now + widget.interval();

                    let state = widget.poll();
                    if last[index].as_ref() == Some(&state) {
                        continue;
                    }
                    last[index] = Some(state.clone());
                    let _ = changes_tx.send_blocking(state);
                }

                std::thread::sleep(TICK);
            }
        })?;

    Ok((WidgetHandle { tx }, changes_rx))
}

impl WidgetHandle {
    pub async fn list(&self) -> Result<Vec<WidgetState>> {
        let (reply_tx, reply_rx) = async_channel::bounded(1);
        self.tx
            .send(Request::List(reply_tx))
            .await
            .map_err(|_| anyhow::anyhow!("widget hub is gone"))?;
        reply_rx
            .recv()
            .await
            .map_err(|_| anyhow::anyhow!("widget hub dropped the reply"))
    }

    /// Make the hub hold these widgets with these settings.
    ///
    /// Called after any write that changed either, which is what makes a
    /// setting take effect without restarting the daemon. Waits for the hub
    /// to be done so that a `ListWidgets` right after a `SetWidgetSetting`
    /// cannot answer with the state from before it.
    pub async fn settle(&self, ids: Vec<String>, settings: WidgetSettings) -> Result<()> {
        let (reply_tx, reply_rx) = async_channel::bounded(1);
        self.tx
            .send(Request::Settle(ids, settings, reply_tx))
            .await
            .map_err(|_| anyhow::anyhow!("widget hub is gone"))?;
        reply_rx
            .recv()
            .await
            .map_err(|_| anyhow::anyhow!("widget hub dropped the reply"))
    }

    pub async fn invoke(&self, id: &str, action: &str) -> Result<()> {
        let (reply_tx, reply_rx) = async_channel::bounded(1);
        self.tx
            .send(Request::Invoke(id.to_string(), action.to_string(), reply_tx))
            .await
            .map_err(|_| anyhow::anyhow!("widget hub is gone"))?;
        reply_rx
            .recv()
            .await
            .map_err(|_| anyhow::anyhow!("widget hub dropped the reply"))
    }
}

pub fn build(ids: &[String], settings: &WidgetSettings) -> Vec<Box<dyn Widget>> {
    ids.iter()
        .filter_map(|id| match id.as_str() {
            "stopwatch" => Some(Box::new(stopwatch::Stopwatch::new()) as Box<dyn Widget>),
            "time-progress" => {
                Some(Box::new(time_progress::TimeProgress::new()) as Box<dyn Widget>)
            }
            "network" => Some(Box::new(network::Network::new()) as Box<dyn Widget>),
            "countdown" => Some(Box::new(countdown::Countdown::new(
                settings.countdown.clone(),
            )) as Box<dyn Widget>),
            "note" => Some(Box::new(note::Note::new(settings.note.clone())) as Box<dyn Widget>),
            "timer" => {
                Some(Box::new(timer::Timer::new(settings.timer.clone())) as Box<dyn Widget>)
            }
            "water" => {
                Some(Box::new(water::Water::new(settings.water.clone())) as Box<dyn Widget>)
            }
            "clock" => Some(Box::new(clock::Clock::new()) as Box<dyn Widget>),
            "battery" => Some(Box::new(battery::Battery::new()) as Box<dyn Widget>),
            "cpu" => Some(Box::new(cpu::Cpu::new()) as Box<dyn Widget>),
            "music" => Some(Box::new(music::Music::new()) as Box<dyn Widget>),
            "pomodoro" => Some(Box::new(pomodoro::Pomodoro::new()) as Box<dyn Widget>),
            unknown => {
                tracing::warn!("ignoring unknown widget {unknown}");
                None
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The list a preferences window offers has to be a list that works.
    ///
    /// `build` shrugs an unknown id off with a warning, so a stale name in
    /// `doca_ipc::WIDGETS` would show up in the window as a widget that can be
    /// added and then never appears. This catches that. The other direction —
    /// a widget built here but missing from the list — no test can see; adding
    /// a widget means adding it to both.
    #[test]
    fn every_widget_on_offer_is_a_widget_that_builds() {
        let ids: Vec<String> = doca_ipc::WIDGETS.iter().map(|id| id.to_string()).collect();

        let built = build(&ids, &WidgetSettings::default());

        assert_eq!(
            built.len(),
            ids.len(),
            "{} of the offered widgets did not build",
            ids.len() - built.len()
        );
    }

    fn ids_of(list: &[&str]) -> Vec<String> {
        list.iter().map(|id| id.to_string()).collect()
    }

    #[test]
    fn a_hub_that_already_holds_what_is_wanted_builds_nothing() {
        let plan = reconcile(&ids_of(&["clock", "timer"]), &ids_of(&["clock", "timer"]));

        assert_eq!(plan, vec![Some(0), Some(1)]);
    }

    /// The reason this function exists: a widget still wanted is carried, not
    /// rebuilt, so adding one to a dock does not reset the timer in another.
    #[test]
    fn an_added_widget_is_the_only_one_built() {
        let plan = reconcile(&ids_of(&["clock", "timer"]), &ids_of(&["clock", "cpu", "timer"]));

        // Indices into what the hub already holds, where `timer` is second —
        // not into the list being asked for, where it is third.
        assert_eq!(plan, vec![Some(0), None, Some(1)]);
    }

    #[test]
    fn a_widget_no_dock_wants_any_more_is_left_out_of_the_plan() {
        let plan = reconcile(&ids_of(&["clock", "timer", "cpu"]), &ids_of(&["cpu"]));

        assert_eq!(plan, vec![Some(2)]);
    }

    #[test]
    fn reordering_carries_every_widget_and_builds_none() {
        let plan = reconcile(&ids_of(&["clock", "timer"]), &ids_of(&["timer", "clock"]));

        assert_eq!(plan, vec![Some(1), Some(0)]);
    }

    /// Two docks can both ask for the clock, but `all_widgets` folds that into
    /// one. If something ever asked twice, the second must not be handed the
    /// same widget the first was.
    #[test]
    fn the_same_widget_is_never_carried_to_two_places_at_once() {
        let plan = reconcile(&ids_of(&["clock"]), &ids_of(&["clock", "clock"]));

        assert_eq!(plan, vec![Some(0), None]);
    }

    #[test]
    fn a_settled_widget_shows_the_setting_that_was_just_written() {
        let settings = WidgetSettings {
            note: crate::config::NoteSettings {
                text: "milk".to_string(),
            },
            ..WidgetSettings::default()
        };
        let mut built = build(&ids_of(&["note"]), &WidgetSettings::default());
        let before = built[0].poll().label;

        built[0].adopt(&settings);

        assert_ne!(before, "milk", "the default already said what the test writes");

        assert_eq!(built[0].poll().label, "milk");
    }

    /// Every widget has to survive being handed settings, including the ten
    /// that take none — the hub calls `adopt` on all of them.
    #[test]
    fn handing_settings_to_every_widget_is_safe() {
        let all: Vec<String> = doca_ipc::WIDGETS.iter().map(|id| id.to_string()).collect();
        let mut built = build(&all, &WidgetSettings::default());

        for widget in built.iter_mut() {
            widget.adopt(&WidgetSettings::default());
            let state = widget.poll();
            assert_eq!(state.id, widget.id(), "a widget changed its own id");
        }
    }

    async fn running(ids: &[&str]) -> WidgetHandle {
        let ids = ids_of(ids);
        let (handle, _changes) = spawn_hub(build(&ids, &WidgetSettings::default()))
            .expect("the hub thread starts");
        handle
    }

    async fn holding(handle: &WidgetHandle) -> Vec<String> {
        handle
            .list()
            .await
            .expect("the hub answers")
            .into_iter()
            .map(|state| state.id)
            .collect()
    }

    /// The pairing between the plan and the widgets being built, which
    /// `reconcile` alone cannot check: taking the built widgets in order from
    /// one `build` of the whole list hands the new slot the widget belonging
    /// to a carried one, and the hub ends up with two clocks and no cpu.
    #[tokio::test]
    async fn an_added_widget_is_the_widget_that_was_added() {
        let hub = running(&["clock"]).await;

        hub.settle(ids_of(&["clock", "cpu"]), WidgetSettings::default())
            .await
            .expect("the hub settles");

        assert_eq!(holding(&hub).await, ids_of(&["clock", "cpu"]));
    }

    #[tokio::test]
    async fn an_id_nobody_wrote_leaves_the_rest_in_their_right_places() {
        let hub = running(&["clock"]).await;

        hub.settle(
            ids_of(&["clock", "teleporter", "cpu"]),
            WidgetSettings::default(),
        )
        .await
        .expect("the hub settles");

        assert_eq!(holding(&hub).await, ids_of(&["clock", "cpu"]));
    }

    #[tokio::test]
    async fn a_widget_no_dock_wants_stops_running() {
        let hub = running(&["clock", "cpu"]).await;

        hub.settle(ids_of(&["cpu"]), WidgetSettings::default())
            .await
            .expect("the hub settles");

        assert_eq!(holding(&hub).await, ids_of(&["cpu"]));
    }

    /// The whole of F4 through the hub rather than through one widget: a
    /// setting written now reaches the widget that is already running, and
    /// `list` cannot answer with the state from before it.
    #[tokio::test]
    async fn a_setting_written_now_is_what_the_running_widget_reports() {
        let hub = running(&["note"]).await;
        let before = hub.list().await.expect("the hub answers")[0].label.clone();

        hub.settle(
            ids_of(&["note"]),
            WidgetSettings {
                note: crate::config::NoteSettings {
                    text: "call the dentist".to_string(),
                },
                ..WidgetSettings::default()
            },
        )
        .await
        .expect("the hub settles");

        let after = hub.list().await.expect("the hub answers")[0].label.clone();
        assert_ne!(before, "call the dentist", "the default said it already");
        assert_eq!(after, "call the dentist");
    }

    #[test]
    fn a_widget_nobody_wrote_is_not_built() {
        let built = build(
            &["clock".to_string(), "teleporter".to_string()],
            &WidgetSettings::default(),
        );

        assert_eq!(built.len(), 1);
    }
}
