use std::sync::{
    LazyLock,
    atomic::{AtomicBool, Ordering},
};

pub struct Flag {
    inner: AtomicBool,
}

/// If true, play a metronome sound at every tick.
pub static USE_METRONOME: LazyLock<Flag> = LazyLock::new(|| Flag::new(false));

impl Flag {
    pub const fn new(initial: bool) -> Self {
        Self {
            inner: AtomicBool::new(initial),
        }
    }

    #[inline]
    pub fn set(&self, value: bool) {
        self.inner.store(value, Ordering::Relaxed);
    }

    #[inline]
    pub fn get(&self) -> bool {
        self.inner.load(Ordering::Relaxed)
    }

    /// Atomically toggles the flag and returns the new value.
    #[inline]
    pub fn toggle(&self) -> bool {
        !self.inner.fetch_not(Ordering::Relaxed)
    }
}
