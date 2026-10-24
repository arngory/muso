//! `muso.conf`: settings in mpv's `key=value` format.
//!
//! ```text
//! # Where to look for music. Repeat the key for more folders.
//! library=~/Music
//! watch=yes
//! replaygain=album
//! # Anything starting with mpv- goes to mpv as is.
//! mpv-audio-device=alsa/default
//! ```
//!
//! A bare key turns a yes/no setting on. A bad line is reported with its line
//! number and skipped, so one typo doesn't throw away the whole file.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// Folders the library is scanned from.
    pub library: Vec<PathBuf>,
    /// Rescan when files in the library folders change.
    pub watch: bool,
    pub gapless: bool,
    pub replaygain: ReplayGain,
    pub start_view: StartView,
    /// How long the controls stay on the cover after the mouse stops.
    pub osc_hide: Duration,
    pub theme: Theme,
    /// `mpv-<option>=<value>` lines, without the prefix, in file order.
    pub mpv: Vec<(String, String)>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            library: Vec::new(),
            watch: true,
            gapless: true,
            replaygain: ReplayGain::Off,
            start_view: StartView::Cover,
            osc_hide: Duration::from_millis(1500),
            theme: Theme::Dark,
            mpv: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReplayGain {
    #[default]
    Off,
    Track,
    Album,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StartView {
    #[default]
    Cover,
    Library,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

/// A line of the file that was skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    /// 1-based.
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "muso.conf:{}: {}", self.line, self.message)
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    /// Reads the file's text over the defaults. `home` replaces a leading `~`
    /// in paths; without it `~` is kept as is.
    pub fn parse(text: &str, home: Option<&Path>) -> (Config, Vec<ConfigError>) {
        let mut config = Config::default();
        let mut errors = Vec::new();
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (key, value) = match line.split_once('=') {
                Some((key, value)) => (key.trim(), Some(value.trim())),
                None => (line, None),
            };
            if let Err(message) = config.set(key, value, home) {
                errors.push(ConfigError {
                    line: n + 1,
                    message,
                });
            }
        }
        (config, errors)
    }

    fn set(&mut self, key: &str, value: Option<&str>, home: Option<&Path>) -> Result<(), String> {
        if let Some(option) = key.strip_prefix("mpv-") {
            if option.is_empty() {
                return Err("mpv- needs an option name after it".into());
            }
            self.mpv
                .push((option.to_owned(), value.unwrap_or("yes").to_owned()));
            return Ok(());
        }
        match key {
            "watch" => self.watch = yes_no(key, value)?,
            "gapless" => self.gapless = yes_no(key, value)?,
            "library" => self.library.push(expand_home(need(key, value)?, home)),
            "replaygain" => {
                self.replaygain = match need(key, value)? {
                    "no" | "off" => ReplayGain::Off,
                    "track" => ReplayGain::Track,
                    "album" => ReplayGain::Album,
                    v => return Err(format!("replaygain is no, track or album, not {v:?}")),
                }
            }
            "start-view" => {
                self.start_view = match need(key, value)? {
                    "cover" => StartView::Cover,
                    "library" => StartView::Library,
                    v => return Err(format!("start-view is cover or library, not {v:?}")),
                }
            }
            "osc-hide" => {
                let v = need(key, value)?;
                self.osc_hide = v
                    .parse::<f64>()
                    .ok()
                    .and_then(|s| Duration::try_from_secs_f64(s).ok())
                    .ok_or_else(|| format!("osc-hide is a number of seconds, not {v:?}"))?;
            }
            "theme" => {
                self.theme = match need(key, value)? {
                    "dark" => Theme::Dark,
                    "light" => Theme::Light,
                    v => return Err(format!("theme is dark or light, not {v:?}")),
                }
            }
            _ => return Err(format!("unknown option {key:?}")),
        }
        Ok(())
    }
}

/// Writes every setting, so the result reads back to the same config.
impl fmt::Display for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let yes = |b: bool| if b { "yes" } else { "no" };
        for path in &self.library {
            writeln!(f, "library={}", path.display())?;
        }
        writeln!(f, "watch={}", yes(self.watch))?;
        writeln!(f, "gapless={}", yes(self.gapless))?;
        let replaygain = match self.replaygain {
            ReplayGain::Off => "no",
            ReplayGain::Track => "track",
            ReplayGain::Album => "album",
        };
        writeln!(f, "replaygain={replaygain}")?;
        let start_view = match self.start_view {
            StartView::Cover => "cover",
            StartView::Library => "library",
        };
        writeln!(f, "start-view={start_view}")?;
        writeln!(f, "osc-hide={}", self.osc_hide.as_secs_f64())?;
        let theme = match self.theme {
            Theme::Dark => "dark",
            Theme::Light => "light",
        };
        writeln!(f, "theme={theme}")?;
        for (option, value) in &self.mpv {
            writeln!(f, "mpv-{option}={value}")?;
        }
        Ok(())
    }
}

fn need<'a>(key: &str, value: Option<&'a str>) -> Result<&'a str, String> {
    match value {
        Some(v) if !v.is_empty() => Ok(v),
        _ => Err(format!("{key} needs a value")),
    }
}

fn yes_no(key: &str, value: Option<&str>) -> Result<bool, String> {
    match value {
        None | Some("yes") => Ok(true),
        Some("no") => Ok(false),
        Some(v) => Err(format!("{key} is yes or no, not {v:?}")),
    }
}

fn expand_home(path: &str, home: Option<&Path>) -> PathBuf {
    match (path.strip_prefix('~'), home) {
        (Some(""), Some(home)) => home.to_owned(),
        (Some(rest), Some(home)) if rest.starts_with('/') => home.join(&rest[1..]),
        _ => PathBuf::from(path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_file_gives_defaults() {
        let (config, errors) = Config::parse("", None);
        assert_eq!(config, Config::default());
        assert!(errors.is_empty());
    }

    #[test]
    fn reads_every_option() {
        let text = "\
# comment
library=~/Music
library = /mnt/nas/music
watch=no
gapless
replaygain=album
start-view=library
osc-hide=2.5
theme=light
mpv-audio-device=alsa/default
mpv-audio-exclusive
";
        let (config, errors) = Config::parse(text, Some(Path::new("/home/a")));
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(
            config.library,
            [
                PathBuf::from("/home/a/Music"),
                PathBuf::from("/mnt/nas/music")
            ]
        );
        assert!(!config.watch);
        assert!(config.gapless);
        assert_eq!(config.replaygain, ReplayGain::Album);
        assert_eq!(config.start_view, StartView::Library);
        assert_eq!(config.osc_hide, Duration::from_millis(2500));
        assert_eq!(config.theme, Theme::Light);
        assert_eq!(
            config.mpv,
            [
                ("audio-device".to_owned(), "alsa/default".to_owned()),
                ("audio-exclusive".to_owned(), "yes".to_owned()),
            ]
        );
    }

    #[test]
    fn bad_lines_are_reported_and_skipped() {
        let text = "theme=blue\nwatch=no\nvolume=50\nosc-hide=-1\nlibrary=\n";
        let (config, errors) = Config::parse(text, None);
        assert!(!config.watch);
        assert_eq!(config.theme, Theme::Dark);
        let lines: Vec<usize> = errors.iter().map(|e| e.line).collect();
        assert_eq!(lines, [1, 3, 4, 5]);
    }

    #[test]
    fn tilde_stays_without_home() {
        let (config, _) = Config::parse("library=~/Music\nlibrary=~bob/x", None);
        assert_eq!(config.library[0], PathBuf::from("~/Music"));
        let (config, _) = Config::parse("library=~bob/x", Some(Path::new("/home/a")));
        assert_eq!(config.library[0], PathBuf::from("~bob/x"));
    }

    #[test]
    fn written_config_reads_back_the_same() {
        let config = Config {
            library: vec!["/music".into(), "/more music".into()],
            watch: false,
            gapless: true,
            replaygain: ReplayGain::Track,
            start_view: StartView::Library,
            osc_hide: Duration::from_millis(750),
            theme: Theme::Light,
            mpv: vec![("af".into(), "loudnorm".into())],
        };
        let (read, errors) = Config::parse(&config.to_string(), None);
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(read, config);
    }
}
