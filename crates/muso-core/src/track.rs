//! Finding audio files and reading what's in them: tags, length and cover.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use lofty::file::TaggedFile;
use lofty::picture::PictureType;
use lofty::prelude::*;

/// File extensions treated as music, lowercase.
const AUDIO_EXTENSIONS: &[&str] = &[
    "aac", "aif", "aiff", "ape", "dff", "dsf", "flac", "m4a", "mka", "mp3", "mpc", "oga", "ogg",
    "opus", "tak", "wav", "wv",
];

/// Image files next to the music that count as its cover, in order of
/// preference, without extension and lowercase.
const COVER_NAMES: &[&str] = &["cover", "folder", "front", "album"];
const COVER_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp"];

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TrackInfo {
    pub path: PathBuf,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub duration: Option<Duration>,
}

impl TrackInfo {
    /// Reads the tags of `path`. A file without tags, or one lofty can't
    /// read, still gets a `TrackInfo` with only the path, so it can be
    /// played and shown by its file name.
    pub fn read(path: &Path) -> TrackInfo {
        let mut info = TrackInfo {
            path: path.to_owned(),
            ..TrackInfo::default()
        };
        let Ok(file) = lofty::read_from_path(path) else {
            return info;
        };
        let duration = file.properties().duration();
        info.duration = (!duration.is_zero()).then_some(duration);
        if let Some(tag) = tag(&file) {
            let text = |s: Option<std::borrow::Cow<'_, str>>| {
                s.map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
            };
            info.title = text(tag.title());
            info.artist = text(tag.artist());
            info.album = text(tag.album());
            info.album_artist = text(tag.get_string(ItemKey::AlbumArtist).map(Into::into));
            info.track_number = tag.track();
            info.disc_number = tag.disk();
        }
        info
    }

    /// The title, or the file name without extension when there is none.
    pub fn display_title(&self) -> String {
        match &self.title {
            Some(title) => title.clone(),
            None => self
                .path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default(),
        }
    }
}

/// The cover of the track at `path` as encoded image bytes: the front cover
/// in its tags, any other picture in them, or an image such as `cover.jpg`
/// in the same folder.
pub fn cover(path: &Path) -> Option<Vec<u8>> {
    embedded_cover(path).or_else(|| folder_cover(path.parent()?))
}

fn embedded_cover(path: &Path) -> Option<Vec<u8>> {
    let file = lofty::read_from_path(path).ok()?;
    let pictures = file.tags().iter().flat_map(|t| t.pictures());
    let front = pictures
        .clone()
        .find(|p| p.pic_type() == PictureType::CoverFront);
    front
        .or_else(|| pictures.clone().next())
        .map(|p| p.data().to_vec())
}

fn folder_cover(dir: &Path) -> Option<Vec<u8>> {
    let mut best: Option<(usize, PathBuf)> = None;
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        let (Some(stem), Some(ext)) = (path.file_stem(), path.extension()) else {
            continue;
        };
        let (stem, ext) = (
            stem.to_string_lossy().to_lowercase(),
            ext.to_string_lossy().to_lowercase(),
        );
        if !COVER_EXTENSIONS.contains(&ext.as_str()) {
            continue;
        }
        let Some(rank) = COVER_NAMES.iter().position(|n| *n == stem) else {
            continue;
        };
        if best.as_ref().is_none_or(|(r, _)| rank < *r) {
            best = Some((rank, path));
        }
    }
    fs::read(best?.1).ok()
}

fn tag(file: &TaggedFile) -> Option<&lofty::tag::Tag> {
    file.primary_tag().or_else(|| file.first_tag())
}

pub fn is_audio(path: &Path) -> bool {
    path.extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .is_some_and(|e| AUDIO_EXTENSIONS.contains(&e.as_str()))
}

/// The audio files among `paths`, with folders replaced by the audio files
/// inside them (recursively, sorted by path). Files given directly are kept
/// in the order given, whatever their extension.
pub fn collect(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for path in paths {
        if path.is_dir() {
            let mut found = Vec::new();
            walk(path, &mut found);
            found.sort();
            files.extend(found);
        } else if path.is_file() {
            files.push(path.clone());
        }
    }
    files
}

fn walk(dir: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // file_type doesn't follow symlinks, so linked folders can't loop.
        match entry.file_type() {
            Ok(t) if t.is_dir() => walk(&path, found),
            Ok(_) if is_audio(&path) => found.push(path),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lofty::config::WriteOptions;
    use lofty::picture::{MimeType, Picture};
    use lofty::tag::{Tag, TagType};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("muso-core-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A second of 8 kHz mono silence.
    fn wav(path: &Path) {
        let data_len = 16_000u32;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        for v in [16u32.to_le_bytes(), [1, 0, 1, 0], 8000u32.to_le_bytes()] {
            bytes.extend_from_slice(&v);
        }
        bytes.extend_from_slice(&16_000u32.to_le_bytes());
        bytes.extend_from_slice(&[2, 0, 16, 0]);
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        bytes.resize(bytes.len() + data_len as usize, 0);
        fs::write(path, bytes).unwrap();
    }

    #[test]
    fn reads_tags_and_embedded_cover() {
        let dir = TempDir::new("tags");
        let path = dir.0.join("01.wav");
        wav(&path);
        let mut tag = Tag::new(TagType::Id3v2);
        tag.set_title("Song".into());
        tag.set_artist("Artist".into());
        tag.set_album("Album".into());
        tag.set_track(3);
        tag.push_picture(
            Picture::unchecked(b"png bytes".to_vec())
                .pic_type(PictureType::CoverFront)
                .mime_type(MimeType::Png)
                .build(),
        );
        tag.save_to_path(&path, WriteOptions::default()).unwrap();

        let info = TrackInfo::read(&path);
        assert_eq!(info.title.as_deref(), Some("Song"));
        assert_eq!(info.artist.as_deref(), Some("Artist"));
        assert_eq!(info.album.as_deref(), Some("Album"));
        assert_eq!(info.track_number, Some(3));
        assert_eq!(info.duration, Some(Duration::from_secs(1)));
        assert_eq!(cover(&path).as_deref(), Some(&b"png bytes"[..]));
    }

    #[test]
    fn untagged_files_fall_back_to_file_name_and_folder_cover() {
        let dir = TempDir::new("untagged");
        let path = dir.0.join("Some Track.wav");
        wav(&path);
        fs::write(dir.0.join("Folder.JPG"), b"folder").unwrap();
        fs::write(dir.0.join("cover.png"), b"cover").unwrap();
        fs::write(dir.0.join("back.jpg"), b"back").unwrap();

        let info = TrackInfo::read(&path);
        assert_eq!(info.title, None);
        assert_eq!(info.display_title(), "Some Track");
        assert_eq!(cover(&path).as_deref(), Some(&b"cover"[..]));
    }

    #[test]
    fn unreadable_files_still_get_a_title() {
        let dir = TempDir::new("broken");
        let path = dir.0.join("broken.flac");
        fs::write(&path, b"not audio").unwrap();
        let info = TrackInfo::read(&path);
        assert_eq!(info.display_title(), "broken");
        assert_eq!(cover(&path), None);
    }

    #[test]
    fn collects_audio_from_folders_in_order() {
        let dir = TempDir::new("collect");
        let album = dir.0.join("Album");
        fs::create_dir_all(album.join("CD2")).unwrap();
        for f in [
            "Album/02.flac",
            "Album/01.FLAC",
            "Album/CD2/01.mp3",
            "Album/cover.jpg",
        ] {
            fs::write(dir.0.join(f), b"").unwrap();
        }
        let loose = dir.0.join("notes.txt");
        fs::write(&loose, b"").unwrap();

        let files = collect(&[loose.clone(), album.clone()]);
        assert_eq!(
            files,
            [
                loose,
                album.join("01.FLAC"),
                album.join("02.flac"),
                album.join("CD2/01.mp3"),
            ]
        );
    }
}
