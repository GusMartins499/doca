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
}

const TICK: Duration = Duration::from_millis(250);

enum Request {
    List(async_channel::Sender<Vec<WidgetState>>),
    Invoke(String, String, async_channel::Sender<()>),
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
