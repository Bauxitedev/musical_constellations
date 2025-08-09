# Debugging/profiling

On Windows to build with extra debug information use this:

```ps
powershell -Command { $env:RUSTFLAGS='-g'; cargo build --lib --release }
```

To run the exported game with more detailed logging, use:

```ps
$env:RUST_LOG = "trace"
./musical_constellations.exe
```

By default, Godot logs to disk in `%appdata%\Godot\app_userdata\` on Windows, even on exported builds. It will not log to disk on web/mobile platforms.

Run the game like `./musical_constellations.exe -- --log-to-godot false` to print to `stdout` instead of Godot's logger (will be faster, but won't appear in Godot's disk logs).
