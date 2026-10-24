//! Finding audio files and reading what's in them: tags, length and cover.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use lofty::file::TaggedFile;
use lofty::picture::PictureType;
use lofty::prelude::*;

/// Which files are music and which images are covers. The core has no list
/// of its own: the playback backend knows what it can play (on desktop,
/// mpv's `audio-exts`, `image-exts` and `cover-art-whitelist`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Formats {
    /// Extensions of playable files, without the dot.
    pub audio: Vec<String>,
    /// Extensions of images that can be a cover.
    pub image: Vec<String>,
    /// File names, without extension, of images next to the music that
    /// count as its cover, most preferred first.
    pub cover_names: Vec<String>,
}

impl Formats {
    pub fn is_audio(&self, path: &Path) -> bool {
        has_extension(path, &self.audio)
    }

    /// The rank of `path` as a cover image, lower is better.
    fn cover_rank(&self, path: &Path) -> Option<usize> {
        if !has_extension(path, &self.image) {
            return None;
        }
        let stem = path.file_stem()?.to_string_lossy();
        self.cover_names
            .iter()
            .position(|n| n.eq_ignore_ascii_case(&stem))
    }
}

fn has_extension(path: &Path, extensions: &[String]) -> bool {
    path.extension()
        .map(|e| e.to_string_lossy())
        .is_some_and(|e| extensions.iter().any(|x| x.eq_ignore_ascii_case(&e)))
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TrackInfo {
    pub path: PathBuf,
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub track_number: Option<u32>,
    pub disc_number: Option<u32>,
    pub year: Option<u32>,
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
            info.year = tag.date().map(|d| d.year.into());
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
/// in its tags, any other picture in them, or an image in the same folder
/// named in [`Formats::cover_names`].
pub fn cover(path: &Path, formats: &Formats) -> Option<Vec<u8>> {
    embedded_cover(path).or_else(|| folder_cover(path.parent()?, formats))
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

fn folder_cover(dir: &Path, formats: &Formats) -> Option<Vec<u8>> {
    let mut best: Option<(usize, PathBuf)> = None;
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        let Some(rank) = formats.cover_rank(&path) else {
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

/// The audio files among `paths`, with folders replaced by the audio files
/// inside them (recursively, sorted by path). Files given directly are kept
/// in the order given, whatever their extension.
pub fn collect(paths: &[PathBuf], formats: &Formats) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for path in paths {
        if path.is_dir() {
            let mut found = Vec::new();
            walk(path, formats, &mut found);
            found.sort();
            files.extend(found);
        } else if path.is_file() {
            files.push(path.clone());
        }
    }
    files
}

pub(crate) fn walk(dir: &Path, formats: &Formats, found: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // file_type doesn't follow symlinks, so linked folders can't loop.
        match entry.file_type() {
            Ok(t) if t.is_dir() => walk(&path, formats, found),
            Ok(_) if formats.is_audio(&path) => found.push(path),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{TempDir, wav};
    use lofty::config::WriteOptions;
    use lofty::picture::{MimeType, Picture};
    use lofty::tag::{Tag, TagType};

    fn formats() -> Formats {
        let list = |s: &str| s.split(',').map(String::from).collect();
        Formats {
            audio: list("flac,mp3,wav"),
            image: list("jpg,png"),
            cover_names: list("cover,folder"),
        }
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
        tag.set_date("1994-03-07".parse().unwrap());
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
        assert_eq!(info.year, Some(1994));
        assert_eq!(info.duration, Some(Duration::from_secs(1)));
        assert_eq!(cover(&path, &formats()).as_deref(), Some(&b"png bytes"[..]));
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
        assert_eq!(cover(&path, &formats()).as_deref(), Some(&b"cover"[..]));
    }

    #[test]
    fn unreadable_files_still_get_a_title() {
        let dir = TempDir::new("broken");
        let path = dir.0.join("broken.flac");
        fs::write(&path, b"not audio").unwrap();
        let info = TrackInfo::read(&path);
        assert_eq!(info.display_title(), "broken");
        assert_eq!(cover(&path, &formats()), None);
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

        let files = collect(&[loose.clone(), album.clone()], &formats());
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
