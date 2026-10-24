# muso

A minimal music player built on [mpv](https://mpv.io).

The window is the album cover; controls show up when you hover, and every
action has a key. Black and white, no clutter.

**Status:** early work, nothing usable yet.

## Layout

- `crates/muso-core`: everything that isn't UI or audio output. The play
  queue with shuffle and repeat, `muso.conf` and `input.conf` in mpv's
  format, and the `Player` trait the playback backends implement.

Planned: a GTK4 client for Linux on libmpv, then Android and Apple clients
that use the system player and reach the core through UniFFI.

## Building

```sh
cargo build --release
cargo test
```

## License

[MPL-2.0](LICENSE)
