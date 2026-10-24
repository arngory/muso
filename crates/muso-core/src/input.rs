//! `input.conf`: key bindings in mpv's format, one `KEY command` per line.
//!
//! ```text
//! RIGHT seek 10
//! Ctrl+f search
//! q ignore
//! ```
//!
//! The user's file goes over the built-in bindings, and `ignore` unbinds a key.
//! Key names follow mpv (`SPACE`, `LEFT`, `Shift+UP`, `Ctrl+,`); clients turn
//! their toolkit's key events into these names.

use std::collections::BTreeMap;
use std::fmt;

/// The built-in bindings, written as an `input.conf`.
pub const DEFAULT_INPUT_CONF: &str = "\
SPACE cycle pause
LEFT seek -5
RIGHT seek 5
UP add volume 2
DOWN add volume -2
< playlist-prev
> playlist-next
m cycle mute
s cycle shuffle
r cycle repeat
t cycle ontop
l show playlists
b show library
q show queue
/ search
o open
n play-next
Shift+UP move up
Shift+DOWN move down
DEL remove
Ctrl+, settings
? help
";

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    CyclePause,
    /// Seconds, relative to the current position.
    Seek(f64),
    /// Percentage points, relative to the current volume.
    AddVolume(i32),
    PlaylistPrev,
    PlaylistNext,
    CycleMute,
    CycleShuffle,
    CycleRepeat,
    CycleOntop,
    Show(View),
    Search,
    Open,
    /// Put the selected track right after the current one.
    PlayNext,
    /// Move the selected row.
    Move(Direction),
    /// Remove the selected row.
    Remove,
    Settings,
    Help,
    /// Does nothing; binding a key to it removes the built-in binding.
    Ignore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Playlists,
    Library,
    Queue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
}

impl std::str::FromStr for Command {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, String> {
        let words: Vec<&str> = s.split_whitespace().collect();
        let command = match words.as_slice() {
            ["cycle", "pause"] => Command::CyclePause,
            ["seek", secs] => Command::Seek(
                secs.parse()
                    .ok()
                    .filter(|s: &f64| s.is_finite())
                    .ok_or_else(|| format!("seek takes seconds, not {secs:?}"))?,
            ),
            ["add", "volume", points] => Command::AddVolume(
                points
                    .parse()
                    .map_err(|_| format!("add volume takes a whole number, not {points:?}"))?,
            ),
            ["playlist-prev"] => Command::PlaylistPrev,
            ["playlist-next"] => Command::PlaylistNext,
            ["cycle", "mute"] => Command::CycleMute,
            ["cycle", "shuffle"] => Command::CycleShuffle,
            ["cycle", "repeat"] => Command::CycleRepeat,
            ["cycle", "ontop"] => Command::CycleOntop,
            ["show", "playlists"] => Command::Show(View::Playlists),
            ["show", "library"] => Command::Show(View::Library),
            ["show", "queue"] => Command::Show(View::Queue),
            ["search"] => Command::Search,
            ["open"] => Command::Open,
            ["play-next"] => Command::PlayNext,
            ["move", "up"] => Command::Move(Direction::Up),
            ["move", "down"] => Command::Move(Direction::Down),
            ["remove"] => Command::Remove,
            ["settings"] => Command::Settings,
            ["help"] => Command::Help,
            ["ignore"] => Command::Ignore,
            _ => return Err(format!("unknown command {s:?}")),
        };
        Ok(command)
    }
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Command::CyclePause => f.write_str("cycle pause"),
            Command::Seek(secs) => write!(f, "seek {secs}"),
            Command::AddVolume(points) => write!(f, "add volume {points}"),
            Command::PlaylistPrev => f.write_str("playlist-prev"),
            Command::PlaylistNext => f.write_str("playlist-next"),
            Command::CycleMute => f.write_str("cycle mute"),
            Command::CycleShuffle => f.write_str("cycle shuffle"),
            Command::CycleRepeat => f.write_str("cycle repeat"),
            Command::CycleOntop => f.write_str("cycle ontop"),
            Command::Show(View::Playlists) => f.write_str("show playlists"),
            Command::Show(View::Library) => f.write_str("show library"),
            Command::Show(View::Queue) => f.write_str("show queue"),
            Command::Search => f.write_str("search"),
            Command::Open => f.write_str("open"),
            Command::PlayNext => f.write_str("play-next"),
            Command::Move(Direction::Up) => f.write_str("move up"),
            Command::Move(Direction::Down) => f.write_str("move down"),
            Command::Remove => f.write_str("remove"),
            Command::Settings => f.write_str("settings"),
            Command::Help => f.write_str("help"),
            Command::Ignore => f.write_str("ignore"),
        }
    }
}

/// mpv's names for keys that aren't a single character.
const SPECIAL_KEYS: &[&str] = &[
    "SPACE",
    "ENTER",
    "ESC",
    "TAB",
    "BS",
    "DEL",
    "INS",
    "HOME",
    "END",
    "PGUP",
    "PGDWN",
    "LEFT",
    "RIGHT",
    "UP",
    "DOWN",
    "SHARP",
    "KP0",
    "KP1",
    "KP2",
    "KP3",
    "KP4",
    "KP5",
    "KP6",
    "KP7",
    "KP8",
    "KP9",
    "KP_DEC",
    "KP_ENTER",
    "PLAY",
    "PAUSE",
    "PLAYPAUSE",
    "STOP",
    "NEXT",
    "PREV",
    "MUTE",
    "VOLUME_UP",
    "VOLUME_DOWN",
];

/// Modifiers in the order mpv writes them.
const MODIFIERS: &[&str] = &["Shift", "Ctrl", "Alt", "Meta"];

/// Spells a key the way mpv does, so `ctrl+shift+left`, `Shift+Ctrl+LEFT`
/// and `Shift+Ctrl+left` are the same binding. Modifiers and special key
/// names ignore case; single characters keep theirs (`a` and `A` differ).
pub fn normalize_key(key: &str) -> String {
    let mut held = [false; 4];
    let mut rest = key;
    'strip: loop {
        for (i, name) in MODIFIERS.iter().enumerate() {
            let len = name.len() + 1;
            let is_prefix = rest.len() > len
                && rest.is_char_boundary(len)
                && rest[..len - 1].eq_ignore_ascii_case(name)
                && rest.as_bytes()[len - 1] == b'+';
            if is_prefix {
                held[i] = true;
                rest = &rest[len..];
                continue 'strip;
            }
        }
        break;
    }
    let mut out = String::new();
    for (name, _) in MODIFIERS.iter().zip(held).filter(|(_, h)| *h) {
        out.push_str(name);
        out.push('+');
    }
    let special = (rest.chars().count() > 1)
        .then(|| {
            SPECIAL_KEYS
                .iter()
                .find(|k| k.eq_ignore_ascii_case(rest))
                .copied()
                .or_else(|| function_key(rest))
        })
        .flatten();
    match special {
        Some(name) => out.push_str(name),
        None => out.push_str(rest),
    }
    out
}

/// `F1` to `F24` in any case.
fn function_key(key: &str) -> Option<&'static str> {
    const NAMES: [&str; 24] = [
        "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12", "F13", "F14",
        "F15", "F16", "F17", "F18", "F19", "F20", "F21", "F22", "F23", "F24",
    ];
    NAMES.iter().find(|n| n.eq_ignore_ascii_case(key)).copied()
}

/// A line of `input.conf` that was skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputError {
    /// 1-based.
    pub line: usize,
    pub message: String,
}

impl fmt::Display for InputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "input.conf:{}: {}", self.line, self.message)
    }
}

impl std::error::Error for InputError {}

#[derive(Debug, Clone, PartialEq)]
pub struct Bindings(BTreeMap<String, Command>);

impl Default for Bindings {
    fn default() -> Self {
        let (bindings, errors) = Bindings::empty().with(DEFAULT_INPUT_CONF);
        debug_assert!(errors.is_empty(), "{errors:?}");
        bindings
    }
}

impl Bindings {
    /// The built-in bindings with the user's `input.conf` over them.
    pub fn parse(text: &str) -> (Bindings, Vec<InputError>) {
        Bindings::default().with(text)
    }

    fn empty() -> Self {
        Bindings(BTreeMap::new())
    }

    fn with(mut self, text: &str) -> (Bindings, Vec<InputError>) {
        let mut errors = Vec::new();
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, command)) = line.split_once(char::is_whitespace) else {
                errors.push(InputError {
                    line: n + 1,
                    message: format!("{line:?} has no command"),
                });
                continue;
            };
            // As in mpv, a `#` after the command starts a comment.
            let command = command.split_once(" #").map_or(command, |(c, _)| c);
            match command.parse() {
                Ok(Command::Ignore) => {
                    self.0.remove(&normalize_key(key));
                }
                Ok(command) => {
                    self.0.insert(normalize_key(key), command);
                }
                Err(message) => errors.push(InputError {
                    line: n + 1,
                    message,
                }),
            }
        }
        (self, errors)
    }

    /// The command bound to `key`, in any spelling [`normalize_key`] accepts.
    pub fn get(&self, key: &str) -> Option<&Command> {
        self.0.get(&normalize_key(key))
    }

    /// Every binding, sorted by key, for the help screen.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &Command)> {
        self.0.iter().map(|(k, c)| (k.as_str(), c))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_cover_the_design() {
        let b = Bindings::default();
        assert_eq!(b.get("SPACE"), Some(&Command::CyclePause));
        assert_eq!(b.get("LEFT"), Some(&Command::Seek(-5.0)));
        assert_eq!(b.get("Ctrl+,"), Some(&Command::Settings));
        assert_eq!(b.get("Shift+DOWN"), Some(&Command::Move(Direction::Down)));
        assert_eq!(b.iter().count(), DEFAULT_INPUT_CONF.lines().count());
    }

    #[test]
    fn user_file_overrides_and_unbinds() {
        let text = "RIGHT seek 10  # longer jump\nq ignore\nCtrl+f search\n";
        let (b, errors) = Bindings::parse(text);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(b.get("RIGHT"), Some(&Command::Seek(10.0)));
        assert_eq!(b.get("q"), None);
        assert_eq!(b.get("Ctrl+f"), Some(&Command::Search));
        assert_eq!(b.get("SPACE"), Some(&Command::CyclePause));
    }

    #[test]
    fn bad_lines_are_reported() {
        let (b, errors) = Bindings::parse("x\nSPACE frobnicate\nUP add volume lots\n");
        let lines: Vec<usize> = errors.iter().map(|e| e.line).collect();
        assert_eq!(lines, [1, 2, 3]);
        assert_eq!(b.get("SPACE"), Some(&Command::CyclePause));
    }

    #[test]
    fn keys_are_spelled_like_mpv() {
        assert_eq!(normalize_key("space"), "SPACE");
        assert_eq!(normalize_key("ctrl+shift+left"), "Shift+Ctrl+LEFT");
        assert_eq!(normalize_key("Alt+Ctrl+a"), "Ctrl+Alt+a");
        assert_eq!(normalize_key("A"), "A");
        assert_eq!(normalize_key("Ctrl++"), "Ctrl++");
        assert_eq!(normalize_key("+"), "+");
        assert_eq!(normalize_key("f12"), "F12");
        assert_eq!(normalize_key("Ctrl+ж"), "Ctrl+ж");
    }

    #[test]
    fn bindings_match_any_spelling() {
        let (b, errors) = Bindings::parse("ctrl+shift+right playlist-next\nspace ignore\n");
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(b.get("Shift+Ctrl+RIGHT"), Some(&Command::PlaylistNext));
        assert_eq!(b.get("SPACE"), None);
    }

    #[test]
    fn commands_read_back_from_their_text() {
        for (_, command) in Bindings::default().iter() {
            assert_eq!(command.to_string().parse::<Command>().as_ref(), Ok(command));
        }
    }
}
