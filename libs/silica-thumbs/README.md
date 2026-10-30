# Procreate Thumbnail Extension for Windows Explorer

Based off Thio's SVG Thumbnail Extension for Windows Explorer.

## Building and registering

From anywhere in the workspace:

```sh
cargo xtask thumbs build        # build target/silica-thumbs/silica_thumbs_x64.dll
cargo xtask thumbs register     # build, then register for the current user (no admin needed)
cargo xtask thumbs unregister   # remove the current-user registration
```

All commands take `--arch x64|x86|arm64` (default `x64`). `register` and `unregister` take
`--machine` to use the machine-wide registration instead, which requires an elevated shell.

Explorer keeps the DLL loaded once it has rendered a thumbnail, so re-registering a rebuilt DLL
can fail to overwrite it. Pass `--restart-explorer` to `register` to release it.

The DLL is built with the `release-dll` profile, which keeps unwinding enabled so panics are
caught before they reach Explorer.
