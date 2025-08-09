use std::sync::{Arc, atomic::Ordering};

use godot::{
    classes::{
        AudioStreamPlayer3D, IStaticBody3D, MeshInstance3D, StandardMaterial3D, StaticBody3D,
        Texture2D, Tween,
        base_material_3d::TextureParam,
        tween::{EaseType, TransitionType},
    },
    prelude::*,
};
use rand::{Rng, seq::IndexedRandom as _};
use rand_xoshiro::Xoshiro256Plus;
use tokio::select;
use tokio_util::sync::CancellationToken;
use tracing::instrument;

use crate::{
    chords::Chord,
    format_gdobj,
    gd::node_stream::{NodalAudioStream, Waveform},
    util::{AtomicF32, LerpSmooth},
};

#[derive(GodotClass)]
#[class(init, base=StaticBody3D)]
pub struct AudioNode {
    base: Base<StaticBody3D>,

    // This will automatically fill in this node in ready(), see https://godot-rust.github.io/docs/gdext/master/godot/prelude/struct.OnReady.html#example---macro-generated-init
    #[init(node = "StreamPlayer")]
    audio_player: OnReady<Gd<AudioStreamPlayer3D>>,

    #[init(node = "Vis")]
    vis: OnReady<Gd<Node3D>>,
    #[init(node = "Vis/Sphere")]
    sphere: OnReady<Gd<MeshInstance3D>>,

    #[init(node = "IndicatorPending")]
    indicator_pending: OnReady<Gd<MeshInstance3D>>,
    #[init(node = "IndicatorCancelling")]
    indicator_cancelling: OnReady<Gd<MeshInstance3D>>,

    mat: Gd<StandardMaterial3D>,
    amplitude_tween: Option<Gd<Tween>>,
    rng: Option<Xoshiro256Plus>,

    #[var]
    chord: Chord,

    #[var]
    semitone_offset: f32,

    #[var]
    octave: i32,

    #[init(val = Arc::new(AtomicF32::new(1.0)))]
    amplitude: Arc<AtomicF32>,
    frequency: Arc<AtomicF32>,

    #[var]
    waveform: Waveform,

    #[init(val = false)]
    active: bool, // True if playing

    #[var]
    duration: f32,

    #[var]
    node_idx: u32, // No longer needed?

    #[var(get, set = set_cancelling)]
    cancelling: bool,

    #[var]
    is_pad: bool,

    #[var]
    color: Color,
    cached_color: Color, // Caches the actual color of the material for perf reasons

    scale: f32,
    cached_scale: f32, // Caches the actual scale of the billboard for perf reasons
}

#[godot_api]
impl IStaticBody3D for AudioNode {
    fn ready(&mut self) {
        let intervals = self.chord.as_intervals();

        //We receive a rng from AudioGraph, so we can safely mutate it without affecting other things, preventing the spread of nondeterminism throughout the codebase
        let mut rng = self.rng.take().expect("please set_rng first");

        let freq = frequency_for_random_note_in_chord(&intervals, self.octave, &mut rng)
            * (self.semitone_offset / 12.0).exp2();
        self.frequency = Arc::new(AtomicF32::new(freq));

        self.audio_player
            .set_stream(&Gd::<NodalAudioStream>::from_init_fn(|_| {
                NodalAudioStream {
                    waveform: self.waveform,
                    frequency: Arc::clone(&self.frequency),
                    amplitude: Arc::clone(&self.amplitude),
                }
            }));

        self.indicator_pending.hide();
        self.indicator_cancelling.hide();

        //Set colors
        let col = {
            let mut col = self.waveform.as_color();

            let brightness = 3.0; // Don't put this too high, or it breaks MSAA on the edges
            col.r *= brightness;
            col.g *= brightness;
            col.b *= brightness;
            col
        };
        self.set_color(col);

        //Cache material
        self.mat = self
            .sphere
            .get_material_override()
            .unwrap()
            .cast::<StandardMaterial3D>();

        self.set_mat_color(self.color);

        //Set texture if pad
        let pad_texture = load::<Texture2D>("res://textures/particle/tri.png"); // Note - it seems to load this once per scene change, so that's good
        if self.is_pad {
            self.set_mat_texture(Gd::clone(&pad_texture));
        }

        //Cache scale
        self.cached_scale =
            (self.vis.get_scale().x + self.vis.get_scale().y + self.vis.get_scale().z) / 3.0;
        self.scale = self.cached_scale;
    }

    fn process(&mut self, delta: f32) {
        //Update scale -> don't call set_scale every frame, it's slow
        let update_scale = true;
        if update_scale {
            let target_scale = if self.active {
                0.1 + self.amplitude.load(Ordering::Relaxed) * 0.7
            } else {
                0.1
            };
            self.scale = self.scale.lerp_smooth(target_scale, 18.0, delta);

            let min_scale_diff = 0.01; // 0.025 looks choppy

            if (self.scale - self.cached_scale).abs() > min_scale_diff {
                self.set_scale(self.scale);
            }
        }

        // Update color -> don't call set_mat_color every frame, it's slow
        let update_color = true;
        if update_color {
            let target_alpha = if self.active { 1.0 } else { 0.1 };
            self.color.a = self.color.a.lerp_smooth(target_alpha, 10.0, delta);

            // We require the alpha to change by this much, before we actually update it on the material (perf optimization)
            // NOTE - this does mean the lerp smooth may not end up exactly at 0.05 (it may converge to a number like 0.07 instead)
            let min_alpha_diff = 0.05;

            if (self.color.a - self.cached_color.a).abs() > min_alpha_diff {
                self.set_mat_color(self.color);
            }
        }
    }
}
#[godot_api]
impl AudioNode {
    #[func]
    pub fn set_cancelling(&mut self, cancelling: bool) {
        self.cancelling = cancelling;
        self.indicator_cancelling.set_visible(cancelling);
    }

    #[func]
    pub fn toggle_cancelling(&mut self) {
        let new_value = !self.get_cancelling();
        self.set_cancelling(new_value);
    }

    pub fn set_rng(&mut self, rng: Xoshiro256Plus) {
        self.rng = Some(rng);
    }
}

impl AudioNode {
    pub fn get_active(&self) -> bool {
        self.active
    }

    pub fn set_playing(&mut self, active: bool) {
        if self.active != active {
            self.active = active;
            self.audio_player.set_playing(active); // This calls start() and stop() on audio_player
        }
    }

    pub fn set_pending(&mut self, pending: bool) {
        self.indicator_pending.set_visible(pending);
    }

    //////////////

    /// Very slow!
    pub fn set_mat_color(&mut self, color: Color) {
        self.mat.set_albedo(color);
        self.cached_color = color; //Cache
    }

    /// Very slow!
    pub fn get_mat_color(&mut self) -> Color {
        self.mat.get_albedo()
    }

    pub fn set_mat_texture(&mut self, texture: Gd<Texture2D>) {
        self.mat.set_texture(TextureParam::ALBEDO, &texture);
    }

    //////////////

    // Very slow!
    pub fn set_scale(&mut self, scale: f32) {
        self.vis.set_scale(Vector3::ONE * scale);
        self.cached_scale = scale; //Cache
    }

    /// Plays the node. (Note - we can't take `&mut self` here, otherwise we get a long-lasting borrow)
    #[cfg_attr(feature = "enable-tracing", instrument(fields(this = format_gdobj!(this))))]
    pub async fn play(this: &mut Gd<Self>, duration_mult: f32, panic_cancel: CancellationToken) {
        // Cancel previous tween if any
        if let Some(mut prevtween) = this.bind_mut().amplitude_tween.take() {
            prevtween.kill(); // Invalidates it and should remove it from the tree, and then drop it because refcounted
            // NOTE - this is called if you use the panic button too!
        }

        this.bind_mut().set_pending(false);

        let final_duration = (this.bind().duration * duration_mult) as f64;
        let amplitude = Arc::clone(&this.bind().amplitude);

        this.bind_mut().set_playing(true);

        let tween_callable = Callable::from_local_fn("", move |args| {
            let value = f32::from_variant(args[0]);

            amplitude.store(value, Ordering::Relaxed);
            Ok(Variant::nil())
        });

        // The tween is bound to `this`, so if `this` gets freed, the tween stops as well.
        let mut tween = this.bind_mut().base_mut().create_tween().unwrap();

        let amp_max = Variant::from(1.0);
        let amp_max_pad = Variant::from(0.5); // Pads are a little less loud than non-pads
        let amp_min = Variant::from(0.0);

        if this.bind().is_pad {
            // Linear pad envelope - attack, sustain and release are all equal (for now)
            let attack = final_duration;
            let sustain = final_duration;
            let release = final_duration;
            tween
                .tween_method(&tween_callable, &amp_min, &amp_max_pad, attack)
                .unwrap()
                .set_ease(EaseType::IN_OUT)
                .unwrap()
                .set_trans(TransitionType::LINEAR)
                .unwrap();

            tween
                .tween_method(&tween_callable, &amp_max_pad, &amp_min, release)
                .unwrap()
                .set_delay(sustain)
                .unwrap()
                .set_ease(EaseType::OUT)
                .unwrap()
                .set_trans(TransitionType::LINEAR)
                .unwrap();
        } else {
            //Quintic plucky envelope
            tween
                .tween_method(&tween_callable, &amp_max, &amp_min, final_duration)
                .unwrap()
                .set_ease(EaseType::OUT)
                .unwrap()
                .set_trans(TransitionType::QUINT)
                .unwrap();
        }

        // Checks if the old tween was None, if not, we have a bug
        let old_tween = this.bind_mut().amplitude_tween.replace(Gd::clone(&tween));
        assert_eq!(old_tween, None);

        let tween_future = tween.signals().finished().to_fallible_future();

        let tween_result = select! {
            result = tween_future => {
                Some(result)
            }
            _ = panic_cancel.cancelled() => {
                //Panic button hit, so stop the sound
                None
            }
        };

        // Only set playing to false if the tween completed all the way without being cancelled halfway through!
        // Otherwise, an earlier play could interrupt a later play.
        match tween_result {
            result @ (Some(Ok(())) | None) => {
                // Tween completed or panic button hit
                this.bind_mut().stop();

                if result.is_none() {
                    // Panic button hit, so stop tween
                    if let Some(tween) = this.bind_mut().amplitude_tween.as_mut() {
                        tween.kill();
                    }
                }

                this.bind_mut().amplitude_tween = None;
            }
            Some(Err(err)) => tracing::warn!("{err}"), // Note - this branch seems never reached, because the future's await is never called
        }
    }

    pub fn stop(&mut self) {
        self.set_playing(false);
    }
}
#[cfg_attr(feature = "enable-tracing", instrument(skip(rng)))]
fn frequency_for_random_note_in_chord<R: Rng>(intervals: &[u8], octave: i32, rng: &mut R) -> f32 {
    // Pick random note from chord
    let note_semitone = *intervals.choose(rng).unwrap() as i32;
    let midi_note = 12 + (12 * octave) + note_semitone;

    440.0 * ((midi_note as f32 - 69.0) / 12.0).exp2()
    //                           ^^^^ Nice
}
