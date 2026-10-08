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
use crate::state::State;

pub trait Widget: Send {
    fn id(&self) -> &str;
    fn interval(&self) -> Duration;
    fn poll(&mut self) -> WidgetState;
    fn invoke(&mut self, _action: &str) {}

    /// Every action this widget's `invoke` answers to.
    ///
    /// Not the same list as `doca_ipc::widget_action::ALL`, which is the
    /// subset a panel draws controls from: this one includes the words that
    /// are only aliases, like a `start` beside a `toggle`. The test in this
    /// module is what holds the two in step, so an action renamed here and
    /// not there fails the suite rather than becoming a button that does
    /// nothing.
    fn actions(&self) -> &'static [&'static str] {
        &[]
    }

    /// Take the settings as they now are, keeping whatever is being counted.
    ///
    /// The alternative — building the widget again from the new settings —
    /// would answer "the water goal is 10 now" by forgetting the six glasses
    /// already drunk and stopping a running timer. Most widgets take no
    /// settings at all and the default no-op is the whole of their answer.
    fn adopt(&mut self, _settings: &WidgetSettings) {}

    /// Take what the day has already accumulated, from disk.
    ///
    /// Called when the widget is built and again when the day turns, and
    /// those are the only two. A widget implementing this never asks what day
    /// it is: the state it is handed has already been carried onto today by
    /// [`crate::state::State::roll_to`], which is the one place that rule
    /// lives.
    fn adopt_state(&mut self, _state: &State) {}

    /// Write down whatever should still be true tomorrow morning.
    ///
    /// Each widget touches only its own corner of the state, so a widget no
    /// dock is showing keeps whatever it had rather than being zeroed by
    /// somebody else's save.
    fn remember(&self, _state: &mut State) {}
}

/// The four things a `Simple` tile draws, read off a state by name.
///
/// Test-only, and deliberately panicking: the tests of the nine widgets that
/// draw one of these were written against a struct with these four fields,
/// and this keeps them saying what they said rather than unwrapping a body in
/// every assertion. A widget whose body stopped being `Simple` would fail
/// them loudly, which is the point.
#[cfg(test)]
pub trait Drawn {
    fn simple(&self) -> doca_ipc::Simple;

    fn label(&self) -> String {
        self.simple().label
    }

    fn detail(&self) -> String {
        self.simple().detail
    }

    fn progress(&self) -> f64 {
        self.simple().progress
    }

    fn active(&self) -> bool {
        self.simple().active
    }
}

#[cfg(test)]
impl Drawn for WidgetState {
    fn simple(&self) -> doca_ipc::Simple {
        match self.body().expect("a body that reads back") {
            doca_ipc::Body::Simple(simple) => simple,
            other => panic!("a {other:?} body does not draw two lines and a bar"),
        }
    }
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

/// Start the thread that polls the widgets, owning the state file with them.
///
/// The store goes in here rather than being shared with the daemon because
/// this thread is the only writer: the counters live in the widgets, and
/// nothing else is in a position to say what they are. One owner is also why
/// no lock is needed around a file that is written on every click.
pub fn spawn_hub(
    mut widgets: Vec<Box<dyn Widget>>,
    mut store: crate::state::Store,
    day: crate::state::Day,
) -> Result<(WidgetHandle, async_channel::Receiver<WidgetState>)> {
    let (tx, rx) = async_channel::unbounded::<Request>();
    let (changes_tx, changes_rx) = async_channel::unbounded::<WidgetState>();

    std::thread::Builder::new()
        .name("widgets".into())
        .spawn(move || {
            let mut last: Vec<Option<WidgetState>> = vec![None; widgets.len()];
            let mut due: Vec<Instant> = vec![Instant::now(); widgets.len()];
            let mut today = store.day();

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
                                // Named here rather than inside each widget's
                                // own `unknown =>` arm, so the complaint can
                                // say what *would* have worked — which is the
                                // difference between "the dock is broken" and
                                // "I misspelled that".
                                if !widgets[index].actions().contains(&action.as_str()) {
                                    tracing::warn!(
                                        "{id} takes no action {action}; it answers to {}",
                                        if widgets[index].actions().is_empty() {
                                            "nothing".to_string()
                                        } else {
                                            widgets[index].actions().join(", ")
                                        }
                                    );
                                    let _ = reply.send_blocking(());
                                    continue;
                                }
                                widgets[index].invoke(&action);
                                due[index] = Instant::now();
                                // A click is the only thing that changes a
                                // counter, so it is the only thing that has
                                // to be written down. `put` compares first,
                                // so an action that counts nothing — a
                                // play/pause — writes nothing.
                                store.put(remembered(&widgets, store.state()));
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
                                        store.state(),
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

                // Midnight, for a machine that has been awake since before
                // it. Checked here and nowhere else: a widget that keeps a
                // count never asks what day it is, it is simply handed the
                // state a new day leaves behind.
                let turned = day.today();
                if turned != today {
                    today = turned;
                    let rolled = store.roll_to(turned).clone();
                    for widget in widgets.iter_mut() {
                        widget.adopt_state(&rolled);
                    }
                    // Whatever that reset is announced now rather than at its
                    // own next tick, which for the water widget is a minute
                    // of showing yesterday's count.
                    for index in 0..widgets.len() {
                        due[index] = Instant::now();
                    }
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

/// The widgets these ids name, each already holding what disk remembers.
///
/// `adopt_state` rather than a constructor argument per widget: a widget that
/// keeps nothing overnight then says nothing about state at all, and the one
/// call site below cannot forget to hand it over.
pub fn build(
    ids: &[String],
    settings: &WidgetSettings,
    state: &State,
) -> Vec<Box<dyn Widget>> {
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
        .map(|mut widget: Box<dyn Widget>| {
            widget.adopt_state(state);
            widget
        })
        .collect()
}

/// The state as it would be written now: what is held, with every running
/// widget's own corner brought up to date.
///
/// Started from the state already held rather than from nothing, because a
/// widget no dock is showing is not here to write its part — and a save that
/// started from nothing would quietly zero it.
fn remembered(widgets: &[Box<dyn Widget>], held: &State) -> State {
    let mut next = held.clone();
    for widget in widgets {
        widget.remember(&mut next);
    }
    next
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A store writing into a directory of this test's own.
    ///
    /// The real path comes from the environment, which every test in this
    /// process shares, so each one names its own file instead.
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("doca-hub-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn store(&self, today: i64) -> crate::state::Store {
            crate::state::Store::open_at(&self.0.join("state.toml"), today)
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    const TODAY: i64 = 20_735;

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

        let built = build(&ids, &WidgetSettings::default(), &State::default());

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

    /// The first line of a widget's tile, for the tests that only care that
    /// a setting arrived.
    fn label_of(widget: &mut Box<dyn Widget>) -> String {
        match widget.poll().body().expect("a body reads back") {
            doca_ipc::Body::Simple(simple) => simple.label,
            other => panic!("a {other:?} has no one label"),
        }
    }

    async fn first_label(hub: &WidgetHandle) -> String {
        let states = hub.list().await.expect("the hub answers");
        match states[0].body().expect("a body reads back") {
            doca_ipc::Body::Simple(simple) => simple.label,
            other => panic!("a {other:?} has no one label"),
        }
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
        let mut built = build(&ids_of(&["note"]), &WidgetSettings::default(), &State::default());
        let before = label_of(&mut built[0]);

        built[0].adopt(&settings);

        assert_ne!(before, "milk", "the default already said what the test writes");

        assert_eq!(label_of(&mut built[0]), "milk");
    }

    /// Every widget has to survive being handed settings, including the ten
    /// that take none — the hub calls `adopt` on all of them.
    #[test]
    fn handing_settings_to_every_widget_is_safe() {
        let all: Vec<String> = doca_ipc::WIDGETS.iter().map(|id| id.to_string()).collect();
        let mut built = build(&all, &WidgetSettings::default(), &State::default());

        for widget in built.iter_mut() {
            widget.adopt(&WidgetSettings::default());
            let state = widget.poll();
            assert_eq!(state.id, widget.id(), "a widget changed its own id");
        }
    }

    async fn running(ids: &[&str]) -> WidgetHandle {
        running_with(ids, Scratch::new("running")).await.0
    }

    /// A hub over a state file of this test's own.
    ///
    /// The day is the real one, not a fixture. A hub notices the turn of the
    /// day on every tick, so a store dated some made-up day is a hub that
    /// rolls it over the moment it starts — which is right, and which cost an
    /// hour before this comment existed.
    async fn running_with(ids: &[&str], scratch: Scratch) -> (WidgetHandle, Scratch) {
        let ids = ids_of(ids);
        let day = crate::state::Day::at_offset(0);
        let store = scratch.store(day.today());
        let widgets = build(&ids, &WidgetSettings::default(), store.state());
        let (handle, _changes) =
            spawn_hub(widgets, store, day).expect("the hub thread starts");
        (handle, scratch)
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
        let before = first_label(&hub).await;

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

        let after = first_label(&hub).await;
        assert_ne!(before, "call the dentist", "the default said it already");
        assert_eq!(after, "call the dentist");
    }

    /// Every control a panel will draw is a control the widget answers to.
    ///
    /// The other half of `doca_ipc::widget_action::ALL`: that list is what a
    /// panel draws buttons from, and a button whose action the widget shrugs
    /// off with "unknown action" is a button that does nothing. Checked
    /// against every widget rather than against a list written out here, so
    /// adding a widget is covered by the same line.
    #[test]
    fn every_action_a_panel_offers_is_one_the_widget_answers_to() {
        let all: Vec<String> = doca_ipc::WIDGETS.iter().map(|id| id.to_string()).collect();
        let built = build(&all, &WidgetSettings::default(), &State::default());

        for (widget, action, _) in doca_ipc::widget_action::ALL {
            let found = built
                .iter()
                .find(|built| built.id() == widget)
                .unwrap_or_else(|| panic!("{widget} is offered {action} and does not exist"));
            assert!(
                found.actions().contains(&action),
                "{widget} is offered {action} and does not answer to it"
            );
        }
    }

    /// And nothing the other way: a widget that answers to something is not
    /// obliged to offer it, but one that offers nothing must answer to
    /// nothing either — otherwise there is a control nobody can reach.
    #[test]
    fn a_widget_that_answers_to_something_offers_it_somewhere() {
        let all: Vec<String> = doca_ipc::WIDGETS.iter().map(|id| id.to_string()).collect();
        let built = build(&all, &WidgetSettings::default(), &State::default());

        for widget in &built {
            if widget.actions().is_empty() {
                continue;
            }
            assert!(
                doca_ipc::widget_action::of(widget.id()).count() > 0,
                "{} takes orders that no panel offers",
                widget.id()
            );
        }
    }

    /// Part of the point of the state file: the glasses counted before the
    /// daemon went down are the glasses it comes back up with.
    #[test]
    fn a_widget_is_built_holding_what_the_day_already_counted() {
        let state = State {
            day: TODAY,
            water: crate::state::WaterState { ml: 4, aimed_at: 2000 },
            closed: Vec::new(),
        };

        let built = build(&ids_of(&["water"]), &WidgetSettings::default(), &state);

        let mut built = built;
        assert_eq!(
            built[0].poll().body().expect("a water body"),
            doca_ipc::Body::Water(doca_ipc::Water { drunk: 4, goal: 2000, bottle: 500, week: vec![0, 0, 0, 0, 0, 0, 4] })
        );
    }

    /// A widget no dock is showing is not there to write its own corner, and
    /// a save must not read that as "nothing counted".
    #[test]
    fn a_save_does_not_zero_a_counter_whose_widget_is_not_running() {
        let held = State {
            day: TODAY,
            water: crate::state::WaterState { ml: 7, aimed_at: 2000 },
            closed: Vec::new(),
        };
        let clock_alone = build(&ids_of(&["clock"]), &WidgetSettings::default(), &held);

        let next = remembered(&clock_alone, &held);

        assert_eq!(next.water.ml, 7, "the water count was wiped by a clock");
    }

    /// The whole of the water half of item 4, through the hub: a click is
    /// written down, and the next daemon starts where this one left off.
    #[tokio::test]
    async fn a_bottle_drunk_now_is_a_bottle_the_next_start_still_knows_about() {
        let today = crate::state::Day::at_offset(0).today();
        let (hub, scratch) = running_with(&["water"], Scratch::new("counted")).await;

        hub.invoke("water", "drink").await.expect("the hub answers");
        hub.invoke("water", "drink").await.expect("the hub answers");
        // `list` waits on the hub thread, so by the time it answers, the
        // invoke before it has been handled and written.
        hub.list().await.expect("the hub answers");

        let next_time = scratch.store(today);
        // Two bottles at the default five hundred millilitres.
        assert_eq!(next_time.state().water.ml, 1000);
    }

    /// And the day after: the file is there, and the count in it is not.
    #[tokio::test]
    async fn a_bottle_drunk_yesterday_is_not_counted_today() {
        let today = crate::state::Day::at_offset(0).today();
        let (hub, scratch) = running_with(&["water"], Scratch::new("yesterday")).await;
        hub.invoke("water", "drink").await.expect("the hub answers");
        hub.list().await.expect("the hub answers");

        let tomorrow = scratch.store(today + 1);

        assert_eq!(tomorrow.state().water.ml, 0);
    }

    #[test]
    fn a_widget_nobody_wrote_is_not_built() {
        let built = build(
            &["clock".to_string(), "teleporter".to_string()],
            &WidgetSettings::default(),
            &State::default(),
        );

        assert_eq!(built.len(), 1);
    }
}
