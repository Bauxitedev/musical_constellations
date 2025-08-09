#![feature(duration_millis_float)]

use godot::prelude::*;
use tracing::instrument;

use crate::{flags::USE_METRONOME, gd::autoload::cli::GAME_ARGS, logging::setup_logging};

pub mod async_node;
pub mod chords;
pub mod flags;
pub mod gd;
pub mod logging;
pub mod profile;
pub mod ui;
pub mod util;

pub mod built_info {
    // built.rs is created by the build script
    include!(concat!(env!("OUT_DIR"), "/built.rs"));
}

struct MusicalConstellationsExtension;

#[gdextension]
unsafe impl ExtensionLibrary for MusicalConstellationsExtension {
    #[cfg_attr(feature = "enable-tracing", instrument)] // Note - this doesn't work, since you setup logging AFTER you enter the function
    fn on_level_init(level: InitLevel) {
        match level {
            InitLevel::Core => (),
            InitLevel::Servers => (),
            InitLevel::Scene => {
                // Little hack - evaluate the GAME_ARGS here to avoid deadlock -> This is bad btw. Maybe NOT log inside of parse_cli_godot_args?
                //     LazyLock::<InnerArgs>::force(&GAME_ARGS);

                setup_logging();
                color_eyre::install().unwrap();

                tracing::info!(USE_METRONOME = USE_METRONOME.get(), "flag");
                tracing::info!(LOG_TO_GODOT = GAME_ARGS.log_to_godot, "flag");
            }
            InitLevel::Editor => (),
        }
    }
}
