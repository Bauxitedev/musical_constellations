use std::{
    f32::consts::TAU,
    sync::{
        Arc, LazyLock,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
};

use colorgrad::Gradient as _;
use godot::{
    classes::{
        AudioServer, AudioStreamPlayback, IAudioStream, IAudioStreamPlayback, native::AudioFrame,
    },
    prelude::*,
};
use rand::{Rng, SeedableRng as _, rngs::SmallRng};
use strum::{EnumIter, IntoEnumIterator};

use crate::{logging::format_as_pointer, util::AtomicF32};

// This file was based on https://github.com/godot-rust/gdext/issues/938

/// Counts the amount of currently active audio streams. Use for profiling.
pub static ACTIVE_STREAMS: LazyLock<AtomicU32> = LazyLock::new(|| AtomicU32::new(0));

#[derive(GodotClass)]
#[class(base=AudioStream, no_init)]
pub struct NodalAudioStream {
    pub waveform: Waveform,
    pub frequency: Arc<AtomicF32>,
    pub amplitude: Arc<AtomicF32>,
}

#[godot_api]
impl IAudioStream for NodalAudioStream {
    fn instantiate_playback(&self) -> Option<Gd<AudioStreamPlayback>> {
        let playback = Gd::<NodalAudioStreamPlayback>::from_init_fn(|_base| {
            ACTIVE_STREAMS.fetch_add(1, Ordering::Relaxed);

            NodalAudioStreamPlayback {
                active: true.into(), // Active true by default, seems to reduce latency!
                sample_rate: AudioServer::singleton().get_mix_rate(), // Seems to be 48khz by default
                sample_index: 0,
                waveform: self.waveform,
                frequency: Arc::clone(&self.frequency),
                amplitude: Arc::clone(&self.amplitude),
                rng: SmallRng::from_os_rng(),
            }
        });

        Some(playback.upcast())
    }
}

#[derive(GodotClass, Debug)]
#[class(base=AudioStreamPlayback, no_init)]
pub struct NodalAudioStreamPlayback {
    active: AtomicBool,
    sample_rate: f32,
    sample_index: usize,
    waveform: Waveform,
    frequency: Arc<AtomicF32>,
    amplitude: Arc<AtomicF32>,
    rng: SmallRng, // Non-portable rng, but it's only used for audio noise generation, so it should be fine.
}

#[godot_api]
impl IAudioStreamPlayback for NodalAudioStreamPlayback {
    // For guidance on implementing this interface, see this example:
    // https://github.com/godotengine/godot/blob/e1b4101e3460dd9c6ba0b7f8d88e9751b8383f5b/modules/vorbis/audio_stream_ogg_vorbis.cpp#L242

    unsafe fn mix_rawptr(
        &mut self,
        buffer: *mut AudioFrame,
        _rate_scale: f32,
        num_requested_frames: i32,
    ) -> i32 {
        if !self.active.load(Ordering::Relaxed) {
            tracing::warn!(
                self = format_as_pointer(self),
                "mix() called on inactive stream"
            );
            return 0;
        }

        self.render_audio(num_requested_frames, buffer)
    }

    fn start(&mut self, _from_pos: f64) {
        self.active.store(true, Ordering::Relaxed);
    }

    fn stop(&mut self) {
        self.active.store(false, Ordering::Relaxed);
    }

    fn is_playing(&self) -> bool {
        self.active.load(Ordering::Relaxed)
    }
}

impl Drop for NodalAudioStreamPlayback {
    fn drop(&mut self) {
        ACTIVE_STREAMS.fetch_sub(1, Ordering::Relaxed); // TODO: maybe use fetch_update instead so you can clamp to 0?
    }
}

impl NodalAudioStreamPlayback {
    fn render_audio(&mut self, num_requested_frames: i32, buffer: *mut AudioFrame) -> i32 {
        let frequency = self.frequency.load(Ordering::Relaxed);
        let amp = 0.1 * self.amplitude.load(Ordering::Relaxed);
        let frac_sample_rate = 1.0 / self.sample_rate;

        // num_requested_frames = 512 (so about 86 calls to render_audio per second per node)

        for i in 0..num_requested_frames {
            if !self.active.load(Ordering::Relaxed) {
                tracing::warn!(
                    self = format_as_pointer(self),
                    "Broke out early at sample {i}"
                );
                return i; // Return the amount of partially processed samples if you return early
            }

            let time = self.sample_index as f32 * frac_sample_rate;

            let sample = amp
                * match self.waveform {
                    Waveform::Sine => {
                        let phase = TAU * frequency * time;
                        phase.sin()
                    }
                    Waveform::Triangle => {
                        4.0 * ((frequency * time + 0.25).fract() - 0.5).abs() - 1.0
                    }
                    Waveform::Saw => 2.0 * (frequency * time).fract() - 1.0,
                    Waveform::Square => {
                        let phase = TAU * frequency * time;
                        let sin = phase.sin();
                        if sin >= 0.0 { 1. } else { -1. }
                    }
                    Waveform::Noise => self.rng.random::<f32>() * 2.0 - 1.0, //-1 ... 1
                };

            // This is the only `unsafe` block in the entire codebase
            unsafe {
                let raw_slot = buffer.offset(i as isize);
                *raw_slot = AudioFrame {
                    left: sample,
                    right: sample,
                };
            }
            self.sample_index += 1;
        }

        num_requested_frames
    }
}

#[derive(Clone, Copy, GodotConvert, Var, Export, Default, Debug, EnumIter, Eq, PartialEq)]
#[godot(via = i64)]
pub enum Waveform {
    Sine,
    #[default]
    Triangle,
    Saw,
    Square,
    Noise,
}

impl Waveform {
    pub fn as_color(&self) -> Color {
        let grad = colorgrad::preset::turbo(); // Very nice color scheme

        match self {
            Waveform::Sine | Waveform::Triangle | Waveform::Saw | Waveform::Square => {
                let progress = (self.to_godot() as f32 + 0.5) / (Waveform::iter().len() - 1) as f32; // We skip the noise waveform in the gradient calculation so -1
                let [r, g, b, a] = grad.at(progress).to_array();
                Color::from_rgba(r, g, b, a)
            }
            Waveform::Noise => Color::GRAY,
        }
    }
}
