use std::{
    sync::LazyLock,
    thread::{self},
    time::{Duration, Instant},
};

use tokio::sync::broadcast;
use tracing::instrument;

#[derive(Debug, Clone, Copy)]
pub struct Tick {
    pub tick: usize, // 0..(ticks_per_beat - 1)
    pub beat: usize, // 0..(beats_per_bar - 1)
    pub bar: usize,  // Current measure index

    pub ticks_per_beat: usize, // 4 (12 in midi)
    pub beats_per_bar: usize,  // Usually 4, 3, etc.

    pub total_ticks: usize,
}

static TICK_CHANNEL: LazyLock<broadcast::Sender<Tick>> = LazyLock::new(|| {
    let (tx, _rx) = broadcast::channel(128); // 128 = capacity per-receiver
    let tx2 = tx.clone();
    thread::spawn(move || beat_emitter(tx2));
    tx // Use subscribe() to get a new receiver
});

/// Use `set_bpm_internal` to send a message on this thread to change the BPM on the next tick.
static BPM_CHANNEL: LazyLock<(flume::Sender<f64>, flume::Receiver<f64>)> =
    LazyLock::new(flume::unbounded);

pub(super) fn set_bpm_internal(new_bpm: f64) {
    let _ = BPM_CHANNEL.0.send(new_bpm);
}

// Synchronous high-precision ticker
#[cfg_attr(feature = "enable-tracing", instrument(skip_all))]
fn beat_emitter(tx: broadcast::Sender<Tick>) {
    let bpm_rx = &BPM_CHANNEL.1;

    let bpm = {
        //Note - The ticker won't start until you call set_bpm_internal at least once
        let span = tracing::info_span!("waiting_for_initial_bpm");
        let _guard = span.enter();

        let result = bpm_rx.recv().unwrap();
        tracing::info!(initial_bpm = result);
        result
    };

    let ticks_per_beat = 4;

    let mut interval = Duration::from_secs_f64(60.0 / bpm / ticks_per_beat as f64);
    let mut deadline = Instant::now();

    let beats_per_bar = 4;

    let mut total_ticks = 0;
    let mut tick = 0;
    let mut beat = 0;
    let mut bar = 0;

    loop {
        // Check for BPM change, throwing away all stale messages
        if let Some(bpm) = bpm_rx.try_iter().last() {
            interval = Duration::from_secs_f64(60.0 / bpm / ticks_per_beat as f64);
            tracing::info!("BPM changed to {bpm}");
        }

        deadline += interval;
        spin_sleep::sleep_until(deadline);

        // Send ticks synchronized to the beat
        let _ = tx.send(Tick {
            tick,
            beat,
            bar,
            total_ticks,
            ticks_per_beat,
            beats_per_bar,
        });

        total_ticks += 1;
        tick += 1;

        if tick >= ticks_per_beat {
            tick = 0;
            beat += 1;

            if beat >= beats_per_bar {
                beat = 0;
                bar += 1;
            }
        }
    }
}

pub struct TickReceiver(broadcast::Receiver<Tick>);

impl TickReceiver {
    pub fn new(sender: &broadcast::Sender<Tick>) -> Self {
        Self(sender.subscribe())
        // From the moment you subscribe, you start receiving ticks (even if you haven't called recv() yet)
        // So, subscribe as early as possible to increate your synchronicity.
    }

    //TODO: maybe impl TickReceiver::clone() with self.0.resubscribe?

    pub async fn wait(&mut self) -> Tick {
        // Normally this would only loop once, unless we lagged
        loop {
            match self.0.recv().await {
                Ok(tick) => return tick,
                Err(broadcast::error::RecvError::Closed) => {
                    panic!("Tick sender dropped, this should never happen")
                }
                Err(broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("Tick receiver lagged and missed {n} ticks, catching up...")
                }
            }
        }
    }
}

pub fn subscribe_to_ticks() -> TickReceiver {
    TickReceiver::new(&TICK_CHANNEL)
}
