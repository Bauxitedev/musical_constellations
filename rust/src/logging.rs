//! This module sets up logging/tracing.
//! Always use `#[cfg_attr(feature = "enable-tracing", instrument)]` instead of `#[instrument]` so we can disable tracing by disabling the `enable-tracing` feature, for performance reasons.
//! You can do this with `cargo build --lib --release --no-default-features` by the way.

use std::{
    borrow::Cow,
    io::{self, Write},
};

use godot::{classes::ProjectSettings, global::godot_print};
use time::macros::format_description;
use tracing::level_filters::LevelFilter;
use tracing_error::ErrorLayer;
use tracing_subscriber::{
    EnvFilter,
    fmt::{MakeWriter, time::LocalTime},
    layer::SubscriberExt as _,
    util::SubscriberInitExt as _,
};

use crate::gd::autoload::cli::GAME_ARGS;

pub fn setup_logging() {
    if cfg!(feature = "enable-tracing") {
        let timer = LocalTime::new(format_description!(
            "[hour]:[minute]:[second].[subsecond digits:3]"
        ));

        let writer = move || -> Box<dyn io::Write> {
            // This closure gets called for every event, so we can change it in real time!
            if GAME_ARGS.log_to_godot {
                Box::new(GodotWriter {}) // GodotWriter is a ZST, so this Box doesn't allocate at all
            } else {
                Box::new(io::stdout())
            }
        };

        let final_filter = get_env_filter();

        let layer = tracing_subscriber::fmt::layer()
            .with_timer(timer)
            .with_writer(writer);
        // Use with_span_events() and FmtSpan::CLOSE to print span duration
        // Or use .with_thread_ids(true) to print thread ids

        tracing_subscriber::registry()
            .with(final_filter)
            .with(layer)
            .with(ErrorLayer::default())
            .init();

        tracing::info!("Tracing enabled");
    } else {
        println!("Tracing disabled");
    }
}

pub fn get_env_filter() -> EnvFilter {
    // NOTE - don't call eyre! in here, or ErrorLayer will panic later

    let settings = ProjectSettings::singleton();
    let godot_rust_log_key = "rust/logging/default_rust_log";

    let default_filter = LevelFilter::ERROR; // Unused if rust/logging/default_rust_log is set in your project settings
    let envvar_filter = EnvFilter::builder().try_from_env();
    let godot_filter = settings
        .has_setting(godot_rust_log_key)
        .then(|| {
            settings
                .get_setting_with_override(godot_rust_log_key) // Important - read the config override for exported games
                .to_string()
        })
        .ok_or("setting missing".to_owned())
        .and_then(|val| {
            if val.is_empty() {
                return Err("empty string".to_owned());
            };
            EnvFilter::builder()
                .parse(val) //empty string is not valid
                .map_err(|err| err.to_string())
        });

    if let Ok(filt) = envvar_filter {
        println!("Using environment filter: '{filt}'");
        filt
    } else if let Ok(filt) = godot_filter {
        println!("Using rust/logging/default_rust_log filter: '{filt}'");
        filt
    } else {
        println!("Using default filter: '{default_filter}'");

        EnvFilter::new(default_filter.to_string())
    }
}
pub struct GodotWriter;

impl Write for GodotWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let s = String::from_utf8_lossy(buf); // Convert raw bytes to string
        let s = remove_trailing_newline(s);
        godot_print!("{}", s); // TODO profile this. If slow, do it in a background thread
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for GodotWriter {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        GodotWriter
    }
}

fn remove_trailing_newline<'a>(input: Cow<'a, str>) -> Cow<'a, str> {
    match input {
        Cow::Owned(owned) => {
            if let Some(stripped) = owned.strip_suffix('\n') {
                Cow::Owned(stripped.to_owned())
            } else {
                Cow::Owned(owned)
            }
        }
        Cow::Borrowed(borrowed) => {
            if let Some(stripped) = borrowed.strip_suffix('\n') {
                Cow::Borrowed(stripped)
            } else {
                Cow::Borrowed(borrowed)
            }
        }
    }
}

//Formats a Gd<T>, by taking its instance ID as hexadecimal
#[macro_export]
macro_rules! format_gdobj {
    ($this:expr) => {
        format_args!("{:#x}", $this.instance_id().to_i64())
    };
}

pub fn format_as_pointer<T>(val: &T) -> String {
    format!("{:#x}", val as *const T as usize)
}
