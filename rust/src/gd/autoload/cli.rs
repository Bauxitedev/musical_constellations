use std::{iter::once, sync::LazyLock};

use clap::{ArgAction, Parser};
use godot::{
    classes::{INode, Node, Os},
    global::godot_print,
    obj::Base,
    prelude::*,
};

use crate::gd::autoload::state_main::parse_hexseed;

pub static GAME_ARGS: LazyLock<InnerArgs> = LazyLock::new(|| parse_cli_godot_args());

// We have a "data bundle" here, see https://godot-rust.github.io/book/register/constructors.html#objects-without-a-base-field
// That means we don't need a `base` field, and we can skip `base = ...` since the default is RefCounted.

/// This is the Godot-exposed wrapped for the `GAME_ARGS` global.
/// If you want to get a CLI arg from GDScript, you can't use that one, it's Rust-only.
/// So, instead, you call `GlobalCliArgs.windowed` to get property `windowed`
/// Note - only the fields that have #[var] are exposed to GDScript.
/// The `seed` for example isn't, because u64 is not supported.
/// To get the seed, use AudioState.get_seed() instead. (this is better anyway, because the seed may change due to user input)
#[derive(Parser, Debug, Clone, GodotClass)]
#[command(version, about = "Musical Constellations", long_about = None)]
#[class(base=Node)]
// Make sure you explicitly set `about` to avoid including the above doc comment in your --help output!
pub struct InnerArgs {
    /// Seed to use (at most 16 hexadecimal characters, e.g. DEADBEEFDEADBEEF)
    #[arg(long, value_parser = parse_hexseed)]
    pub seed: Option<u64>,

    /// Skip intro animation
    #[arg(long)]
    #[var]
    pub skip_intro: bool,

    /// Don't start in fullscreen
    #[arg(long)]
    #[var]
    pub windowed: bool,

    /// If true, tracing::info and related macros will log using godot_print. If false, they use stdout.
    /// Must be true to use Godot's log-to-disk functionality.
    #[arg(
        long,
        action = ArgAction::Set,
        default_value_t = true,
        default_missing_value = "true", // Somehow clap has this option not properly supported in derive, so it needs to be a string
        num_args = 0..=1,
        require_equals = false,
    )]
    #[var]
    // Note - need some hacks to get this to work, since Clap interprets booleans as a special case compared to any other value.
    // See https://github.com/clap-rs/clap/issues/1649#issuecomment-2144879038
    // Also, don't use a doc comment here, this comment should be hidden from the user.
    pub log_to_godot: bool,
}

impl Default for InnerArgs {
    // TODO - this creates a discrepancy between the InnerArgs::default() used when the CLI args are invalid, and the default values Clap uses when the CLI args ARE valid.
    fn default() -> Self {
        Self {
            seed: None,
            skip_intro: false,
            windowed: false,
            log_to_godot: true,
        }
    }
}

#[godot_api]
impl INode for InnerArgs {
    fn init(_base: Base<Node>) -> Self {
        // Note - this clones the InnerArgs, but it doesn't contain a lot of data, so should be fine
        GAME_ARGS.clone()
    }
}

pub fn parse_cli_godot_args() -> InnerArgs {
    // Make sure you run the game like `game.exe -- --seed DEAD --windowed --skip-intro`
    // ! WARNING - Avoid using tracing::*! in this method, it immediately causes a deadlock w.r.t. GAME_ARGS + seems to mess up the interleaving of the log messages, making it hard to read.

    let cli_args = Os::singleton().get_cmdline_user_args();
    godot_print!("cli_args: {:?}", cli_args.as_slice());

    // Little hack - inject the name of the executable into the `cli_args`, followed by --, to avoid passing the args to godot instead of the game
    // The -- is purely a cosmetic thing here, so you get better error messages from Clap.
    let cli_args = once("musical_constellations --".to_owned())
        .chain(cli_args.as_slice().iter().map(|str| str.to_string()));

    match InnerArgs::try_parse_from(cli_args) {
        Ok(args) => {
            godot_print!(">>> CLI args parsed: {args:?}");
            args
        }
        Err(err) => {
            use clap::error::ErrorKind;

            // if the errortype is DisplayHelp, DisplayHelpOnMissingArgumentOrSubcommand, or DisplayVersion, do not print an error, instead just print the help
            let is_fake_error = match err.kind() {
                ErrorKind::DisplayHelp => true,
                ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => true,
                ErrorKind::DisplayVersion => true,
                _ => false,
            };

            if is_fake_error {
                godot_print!(">>> {err}");
            } else {
                godot_print!("Failed to parse CLI args: {err}");
            }

            // Use default GameArgs if invalid
            let default = InnerArgs::default();
            godot_print!("Using default InnerArgs: {:?}", default);
            default
        }
    }
}
