use std::{cell::OnceCell, sync::atomic::Ordering};

use godot::{classes::Engine, prelude::*};
use tracing::instrument;

use crate::{
    built_info,
    gd::{
        autoload::{cli::GAME_ARGS, state_tick::set_bpm_internal},
        node_stream::ACTIVE_STREAMS,
    },
};

thread_local! {
    pub static AUDIOSTATE_AUTOLOAD_NODEPATH: OnceCell<NodePath> = const { OnceCell::new() };
    //Note: we can't store the Node itself in a global, since it's immutable.
    //Note2: you can just do get_node("/root/Frac3dAutoload") instead of storing the NodePath.
}

#[derive(GodotClass, Debug)]
#[class(init, base=Node)]
pub struct AudioState {
    #[base]
    base: Base<Node>,

    #[init(val = 115.0)] // Note: if this value is smaller than the hslider min, it will emit twice.
    #[var(get, set = set_bpm)]
    bpm: f64,

    #[init]
    #[var(get, set = set_seed)]
    seed: i64, // u64 not supported :(

    #[var(get, set=set_graph_debug_str)]
    graph_debug_str: GString,
}

#[godot_api]
impl INode for AudioState {
    fn ready(&mut self) {
        self.set_bpm(self.bpm); // This triggers signal + atomic, which starts the ticker

        if let Some(cli_seed) = GAME_ARGS.seed {
            self.set_seed(cli_seed as i64); // Bitwise conversion
        } else {
            self.set_seed(0xDEADBEEF); //3735928559
        }
        //Store the nodepath of this node
        AUDIOSTATE_AUTOLOAD_NODEPATH.with(|cell| {
            cell.set(self.base().get_path())
                .expect("AUDIOSTATE_AUTOLOAD_NODEPATH initialized twice")
        });
    }
}

#[godot_api]
impl AudioState {
    #[signal]
    fn bpm_changed(bpm: f64);
    #[signal]
    fn seed_changed(seed: i64);
    #[signal]
    fn graph_debug_str_changed(graph_debug_str: GString);

    /// Gets the autoload instance of this node.
    pub fn autoload() -> Gd<Self> {
        AUDIOSTATE_AUTOLOAD_NODEPATH.with(|nodepath| {
            let nodepath = nodepath.get().expect(
                "AUDIOSTATE_AUTOLOAD_NODEPATH missing, ensure you're calling from the main thread",
            );
            Engine::singleton()
                .get_main_loop()
                .unwrap()
                .cast::<SceneTree>()
                .get_root()
                .unwrap()
                .get_node_as::<Self>(nodepath)
        })
    }

    #[func]
    pub fn set_bpm(&mut self, bpm: f64) {
        set_bpm_internal(bpm);
        self.bpm = bpm;

        self.signals().bpm_changed().emit(bpm);
    }

    #[func]
    /// Sets the seed from a string. Returns false if parsing the string failed.
    #[cfg_attr(feature = "enable-tracing", instrument(skip(self)))]
    pub fn set_seed_str(&mut self, seed_str: String) -> bool {
        tracing::info!(seed_str);

        // This rejects negative numbers automatically
        match parse_hexseed(&seed_str) {
            Ok(seed) => {
                self.set_seed(seed as i64); // Raw bit conversion
                true
            }
            Err(err) => {
                tracing::error!(%err, "seed_str parse failed");
                false
            }
        }
    }

    #[func]
    #[cfg_attr(feature = "enable-tracing", instrument(skip(self)))]
    pub fn set_seed(&mut self, seed: i64) {
        if seed != self.seed {
            self.seed = seed;

            self.signals().seed_changed().emit(seed);
        }
    }

    #[func]
    pub fn get_seed_str(&self) -> String {
        format!("{:016X}", self.seed) //Format as 16 chars with 0 padding
    }

    #[func]
    pub fn randomize_seed(&mut self) {
        self.seed = rand::random();
        //Note - this does not trigger the signal
    }

    /// Get the performance string, shown on the bottom-left.
    #[func]
    pub fn get_perf_str(&self) -> String {
        format!(
            "{:>3} FPS\n{:>3} playing streams\n{:>3} active tweens",
            Engine::singleton().get_frames_per_second(),
            ACTIVE_STREAMS.load(Ordering::Relaxed),
            self.base().get_tree().unwrap().get_processed_tweens().len(),
        )
    }

    /// Get the debugging string, shown on the Statistics tab.
    #[func]
    pub fn get_debug_str(&self) -> String {
        format!(
            "Statistics\n----------------------\n{}",
            (self.get_graph_debug_str())
        )
    }

    #[func]
    pub fn set_graph_debug_str(&mut self, graph_debug_str: GString) {
        self.graph_debug_str = GString::clone(&graph_debug_str); // Cheap clone (refcounted)
        self.signals()
            .graph_debug_str_changed()
            .emit(&graph_debug_str);
    }

    #[func]
    pub fn get_version_str(&self) -> String {
        format!(
            r#"About
----------------------
Crate: {}
Version: {} (commit {})
Features: {:?}
----------------------
Build date: {}
Build target: {}
CI: {}
{}"#,
            built_info::PKG_NAME,
            built_info::PKG_VERSION,
            built_info::GIT_COMMIT_HASH_SHORT.unwrap_or("???"),
            built_info::FEATURES_LOWERCASE,
            built_info::BUILT_TIME_UTC, // Honors the environment variable SOURCE_DATE_EPOCH for reproducible builds
            built_info::TARGET,
            if built_info::CI_PLATFORM.is_some() {
                "✅"
            } else {
                "❌"
            },
            built_info::RUSTC_VERSION,
        )
    }
}

pub fn parse_hexseed(s: &str) -> Result<u64, std::num::ParseIntError> {
    u64::from_str_radix(s, 16)
}
