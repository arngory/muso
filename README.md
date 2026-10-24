# muso

A minimal music player built on [mpv](https://mpv.io).

The window is the album cover; controls show up when you hover, and every
action has a key. Black and white, no clutter.

**Status:** early work. It plays files and folders you open or drop on the
window and has a library of your music folders with search; there are no
playlists or settings screen yet.

## Install

Releases have a tarball with the `muso` binary for x86_64 Linux. It needs
GTK 4.10 or newer, libmpv 0.35 or newer, SQLite 3 and glibc 2.39 (Ubuntu
24.04, Fedora 40, Arch and newer):

```sh
# Debian, Ubuntu
sudo apt install libgtk-4-1 libmpv2 libsqlite3-0
# Fedora
sudo dnf install gtk4 mpv-libs sqlite-libs
# Arch
sudo pacman -S gtk4 mpv sqlite

tar xzf muso-*-x86_64-linux.tar.gz
sudo install -Dm755 muso-*/muso /usr/local/bin/muso
```

## Use

```sh
muso ~/Music/Album        # or files, or nothing and press O
```

Move the mouse over the cover to see the controls. Keys follow mpv: space
pauses, ← → seek, ↑ ↓ change the volume, < > switch tracks, s shuffles,
r cycles repeat, m mutes, ? lists every key, the wheel changes the volume.
A double click goes fullscreen. The controls grow with the window, as in mpv,
and follow the desktop's text scale.

b opens the library: artists, their albums and the album's tracks, with the
controls in a bar below. / searches it (↑ ↓ pick a track, Enter plays,
Shift+Enter adds it to the queue, Esc closes), q shows the queue, n plays
the selected track next, and b again goes back to the cover. A double click
on an album or a track plays it. The library is your `library` folders, or
the desktop's Music folder if none are set. muso reads their tags once into
`~/.local/share/muso/library.db` and, with `watch=yes`, notices new and
changed files while it runs.

The interface speaks your desktop's language when there is a translation
(English and Russian so far). It plays whatever mpv calls audio
(`audio-exts`) and takes the folder picture mpv would (`image-exts`,
`cover-art-whitelist`); change those with `mpv-` options.

Settings live in `~/.config/muso/muso.conf` and keys in
`~/.config/muso/input.conf`, both in mpv's format:

```ini
# muso.conf
# The library's folders; repeat for more. The Music folder if not set.
library=~/Music
# Rescan when files in them change.
watch=yes
# Open in the library instead of the cover: cover or library.
start-view=cover
gapless=yes
# no, track or album
replaygain=album
# Seconds before the controls hide.
osc-hide=1.5
# Milliseconds messages such as the volume stay up.
osd-duration=1000
# dark or light
theme=dark
# Any mpv option, prefixed with mpv-.
mpv-audio-device=alsa/default
```

```ini
# input.conf
RIGHT seek 10
q ignore
```

The defaults are in [`crates/muso-core/input.conf`](crates/muso-core/input.conf).

## Layout

- `crates/muso-core`: everything that isn't UI or audio output. The play
  queue with shuffle and repeat, tags and covers, the library file and
  folder watching, `muso.conf` and `input.conf`, and the `Player` trait the
  playback backends implement.
- `crates/muso-mpv`: `Player` on libmpv, with gapless switching.
- `crates/muso-gtk`: the Linux client, built as the `muso` binary. Icons
  are SVG files in `icons/`, translations are gettext files in `po/`.

Planned: Android and Apple clients that use the system player and reach the
core through UniFFI.

## Building

Needs Rust 1.92 and the GTK 4, libmpv and SQLite development packages
(`libgtk-4-dev libmpv-dev libsqlite3-dev` on Debian and Ubuntu).

```sh
cargo build --release     # target/release/muso
cargo test
```

## Translating

Copy `crates/muso-gtk/po/muso.pot` to `po/<language>.po` (for example
`de.po`), fill in the `msgstr` lines and rebuild; the file is built into the
binary. `cargo test` fails when a string in the code has no entry in a
catalog, or a catalog has entries the code no longer uses.

## Releasing

Bump `version` in `Cargo.toml`, merge, then push a matching tag:

```sh
git tag v0.2.0 && git push origin v0.2.0
```

CI builds the tarball and publishes the GitHub release.

## License

[MPL-2.0](LICENSE)
