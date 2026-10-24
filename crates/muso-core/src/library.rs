//! The music library: every audio file under the `library` folders with its
//! tags, kept in an SQLite file so a start doesn't read them all again. A
//! scan reads only the files that are new or changed since the last one.
//!
//! Tracks group into albums by album artist (or artist) and album title.
//! A track without an album tag belongs to an album named after its folder,
//! and one without any artist to the artist `""`, which clients show as
//! "unknown".

use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, Row, ToSql, params};

use crate::track::{self, Formats, TrackInfo};

pub use rusqlite::Error;

/// Bumped when the table changes; an older file is then read again from
/// scratch, which is only slow once.
const SCHEMA_VERSION: i32 = 1;
/// Files read between commits; a scan reports progress after each batch.
const BATCH: usize = 100;

pub struct Library {
    db: Connection,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artist {
    /// `""` for tracks without an artist.
    pub name: String,
    pub albums: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Album {
    pub artist: String,
    pub title: String,
    /// The earliest year among its tracks.
    pub year: Option<u32>,
    pub tracks: u32,
    /// The first track, to take the cover from.
    pub first: PathBuf,
}

/// How far a scan got: `done` of the `total` new or changed files are read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Progress {
    pub done: usize,
    pub total: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScanStats {
    /// Files read because they were new or changed.
    pub read: usize,
    /// Tracks dropped because their file is gone.
    pub removed: usize,
}

/// Search results, each group in library order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Found {
    pub artists: Vec<Artist>,
    pub albums: Vec<Album>,
    pub tracks: Vec<TrackInfo>,
}

impl Found {
    pub fn is_empty(&self) -> bool {
        self.artists.is_empty() && self.albums.is_empty() && self.tracks.is_empty()
    }
}

const TRACK_COLUMNS: &str =
    "path, title, artist, album, album_artist, track, disc, year, duration_ms";
const TRACK_ORDER: &str = "COALESCE(disc, 0), COALESCE(track, 0), path";

impl Library {
    /// Opens the library file at `path`, creating it and its folder if
    /// needed.
    pub fn open(path: &Path) -> Result<Library, Error> {
        if let Some(dir) = path.parent() {
            // Opening reports the error if this fails.
            let _ = fs::create_dir_all(dir);
        }
        let db = Connection::open(path)?;
        // A scan in another thread writes while the window reads.
        db.execute_batch("PRAGMA journal_mode = WAL")?;
        Library::setup(db)
    }

    /// A library that lives only as long as the value, for tests.
    pub fn in_memory() -> Result<Library, Error> {
        Library::setup(Connection::open_in_memory()?)
    }

    fn setup(db: Connection) -> Result<Library, Error> {
        db.busy_timeout(Duration::from_secs(10))?;
        let version: i32 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
        if version != SCHEMA_VERSION {
            db.execute_batch(&format!(
                "DROP TABLE IF EXISTS tracks;
                 CREATE TABLE tracks (
                     path BLOB PRIMARY KEY,
                     mtime INTEGER NOT NULL,
                     size INTEGER NOT NULL,
                     title TEXT,
                     artist TEXT,
                     album TEXT,
                     album_artist TEXT,
                     track INTEGER,
                     disc INTEGER,
                     year INTEGER,
                     duration_ms INTEGER,
                     group_artist TEXT NOT NULL,
                     group_album TEXT NOT NULL,
                     find_artist TEXT NOT NULL,
                     find_album TEXT NOT NULL,
                     find_track TEXT NOT NULL
                 );
                 CREATE INDEX tracks_album ON tracks (group_artist, group_album);
                 PRAGMA user_version = {SCHEMA_VERSION};"
            ))?;
        }
        Ok(Library { db })
    }

    /// Brings the library in line with the audio files under `roots`:
    /// reads new and changed files and drops tracks whose file is gone.
    /// Tracks under a root that can't be reached (an unmounted drive) are
    /// kept. `progress` hears how far reading has got.
    pub fn scan(
        &mut self,
        roots: &[PathBuf],
        formats: &Formats,
        mut progress: impl FnMut(Progress),
    ) -> Result<ScanStats, Error> {
        let mut files = Vec::new();
        for root in roots {
            track::walk(root, formats, &mut files);
        }

        let mut known: HashMap<Vec<u8>, (i64, i64)> = HashMap::new();
        {
            let mut stmt = self.db.prepare("SELECT path, mtime, size FROM tracks")?;
            let rows = stmt.query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?;
            for row in rows {
                let (path, stamp) = row?;
                known.insert(path, stamp);
            }
        }

        let mut stale = Vec::new();
        for path in files {
            let Some(now) = stamp(&path) else { continue };
            if known.remove(path.as_os_str().as_bytes()) != Some(now) {
                stale.push((path, now));
            }
        }
        let unreachable: Vec<&PathBuf> = roots.iter().filter(|r| !r.is_dir()).collect();
        let gone: Vec<Vec<u8>> = known
            .into_keys()
            .filter(|p| !unreachable.iter().any(|r| bytes_path(p).starts_with(r)))
            .collect();

        let tx = self.db.transaction()?;
        {
            let mut delete = tx.prepare("DELETE FROM tracks WHERE path = ?1")?;
            for path in &gone {
                delete.execute([path])?;
            }
        }
        tx.commit()?;

        let total = stale.len();
        progress(Progress { done: 0, total });
        let mut done = 0;
        for batch in stale.chunks(BATCH) {
            let tx = self.db.transaction()?;
            {
                let mut insert = tx.prepare_cached(
                    "INSERT OR REPLACE INTO tracks (
                         path, mtime, size, title, artist, album, album_artist,
                         track, disc, year, duration_ms,
                         group_artist, group_album, find_artist, find_album, find_track
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                )?;
                for (path, (mtime, size)) in batch {
                    let info = TrackInfo::read(path);
                    let artist = group_artist(&info);
                    let album = group_album(&info);
                    let find_artist = artist.to_lowercase();
                    let find_album = format!(
                        "{find_artist}\n{}\n{}",
                        info.artist.as_deref().unwrap_or_default().to_lowercase(),
                        album.to_lowercase()
                    );
                    let find_track =
                        format!("{find_album}\n{}", info.display_title().to_lowercase());
                    insert.execute(params![
                        path.as_os_str().as_bytes(),
                        mtime,
                        size,
                        info.title,
                        info.artist,
                        info.album,
                        info.album_artist,
                        info.track_number,
                        info.disc_number,
                        info.year,
                        info.duration.map(|d| d.as_millis() as i64),
                        artist,
                        album,
                        find_artist,
                        find_album,
                        find_track,
                    ])?;
                }
            }
            tx.commit()?;
            done += batch.len();
            progress(Progress { done, total });
        }
        Ok(ScanStats {
            read: total,
            removed: gone.len(),
        })
    }

    pub fn track_count(&self) -> Result<u64, Error> {
        self.db
            .query_row("SELECT COUNT(*) FROM tracks", [], |r| r.get::<_, i64>(0))
            .map(|n| n as u64)
    }

    /// Every artist, by name, the unknown one last.
    pub fn artists(&self) -> Result<Vec<Artist>, Error> {
        self.find_artists("", &[], None)
    }

    /// The albums of `artist`, or of everyone, oldest first.
    pub fn albums(&self, artist: Option<&str>) -> Result<Vec<Album>, Error> {
        match artist {
            Some(a) => self.find_albums("WHERE group_artist = ?1", &[&a], None),
            None => self.find_albums("", &[], None),
        }
    }

    /// The tracks of an album, by disc and number.
    pub fn tracks(&self, album: &Album) -> Result<Vec<TrackInfo>, Error> {
        self.find_tracks(
            "WHERE group_artist = ?1 AND group_album = ?2",
            &[&album.artist, &album.title],
            None,
        )
    }

    /// The track at `path`, as last read.
    pub fn track(&self, path: &Path) -> Result<Option<TrackInfo>, Error> {
        self.db
            .prepare_cached(&format!(
                "SELECT {TRACK_COLUMNS} FROM tracks WHERE path = ?1"
            ))?
            .query_row([path.as_os_str().as_bytes()], track_info)
            .optional()
    }

    /// Every track, artist by artist and album by album, as the library
    /// view lists them.
    pub fn all_tracks(&self) -> Result<Vec<PathBuf>, Error> {
        let mut all = Vec::new();
        for album in self.albums_in_order()? {
            all.extend(self.tracks(&album)?.into_iter().map(|t| t.path));
        }
        Ok(all)
    }

    fn albums_in_order(&self) -> Result<Vec<Album>, Error> {
        let mut albums = self.albums(None)?;
        let rank: HashMap<String, usize> = self
            .artists()?
            .into_iter()
            .enumerate()
            .map(|(i, a)| (a.name, i))
            .collect();
        // Stable, so each artist's albums stay oldest first.
        albums.sort_by_key(|a| rank.get(&a.artist).copied());
        Ok(albums)
    }

    /// Artists, albums and tracks whose names hold every word of `query`,
    /// ignoring case. Albums and tracks match on their artist too. Each
    /// group has at most `limit` entries.
    pub fn search(&self, query: &str, limit: usize) -> Result<Found, Error> {
        let words: Vec<String> = query
            .to_lowercase()
            .split_whitespace()
            .map(|w| format!("%{}%", escape_like(w)))
            .collect();
        if words.is_empty() {
            return Ok(Found::default());
        }
        let params: Vec<&dyn ToSql> = words.iter().map(|w| w as &dyn ToSql).collect();
        let filter = |column: &str| {
            let terms: Vec<String> = (1..=words.len())
                .map(|i| format!("{column} LIKE ?{i} ESCAPE '\\'"))
                .collect();
            format!("WHERE {}", terms.join(" AND "))
        };
        Ok(Found {
            artists: self.find_artists(&filter("find_artist"), &params, Some(limit))?,
            albums: self.find_albums(&filter("find_album"), &params, Some(limit))?,
            tracks: self.find_tracks(&filter("find_track"), &params, Some(limit))?,
        })
    }

    fn find_artists(
        &self,
        filter: &str,
        params: &[&dyn ToSql],
        limit: Option<usize>,
    ) -> Result<Vec<Artist>, Error> {
        let mut stmt = self.db.prepare_cached(&format!(
            "SELECT group_artist, COUNT(DISTINCT group_album) FROM tracks {filter}
             GROUP BY group_artist"
        ))?;
        let mut artists = stmt
            .query_map(params, |r| {
                Ok(Artist {
                    name: r.get(0)?,
                    albums: r.get(1)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        // SQLite only folds ASCII case; names are often not ASCII.
        artists.sort_by_cached_key(|a| (a.name.is_empty(), a.name.to_lowercase()));
        if let Some(limit) = limit {
            artists.truncate(limit);
        }
        Ok(artists)
    }

    fn find_albums(
        &self,
        filter: &str,
        params: &[&dyn ToSql],
        limit: Option<usize>,
    ) -> Result<Vec<Album>, Error> {
        let mut stmt = self.db.prepare_cached(&format!(
            "SELECT group_artist, group_album, MIN(year), COUNT(*),
                 (SELECT path FROM tracks f
                  WHERE f.group_artist = t.group_artist AND f.group_album = t.group_album
                  ORDER BY {TRACK_ORDER} LIMIT 1)
             FROM tracks t {filter}
             GROUP BY group_artist, group_album
             ORDER BY MIN(year) IS NULL, MIN(year), group_album
             {}",
            limit_clause(limit)
        ))?;
        stmt.query_map(params, |r| {
            Ok(Album {
                artist: r.get(0)?,
                title: r.get(1)?,
                year: r.get(2)?,
                tracks: r.get(3)?,
                first: bytes_path(&r.get::<_, Vec<u8>>(4)?),
            })
        })?
        .collect()
    }

    fn find_tracks(
        &self,
        filter: &str,
        params: &[&dyn ToSql],
        limit: Option<usize>,
    ) -> Result<Vec<TrackInfo>, Error> {
        let mut stmt = self.db.prepare_cached(&format!(
            "SELECT {TRACK_COLUMNS} FROM tracks {filter}
             ORDER BY group_artist, group_album, {TRACK_ORDER} {}",
            limit_clause(limit)
        ))?;
        stmt.query_map(params, track_info)?.collect()
    }
}

fn track_info(r: &Row<'_>) -> Result<TrackInfo, Error> {
    Ok(TrackInfo {
        path: bytes_path(&r.get::<_, Vec<u8>>(0)?),
        title: r.get(1)?,
        artist: r.get(2)?,
        album: r.get(3)?,
        album_artist: r.get(4)?,
        track_number: r.get(5)?,
        disc_number: r.get(6)?,
        year: r.get(7)?,
        duration: r
            .get::<_, Option<i64>>(8)?
            .map(|ms| Duration::from_millis(ms as u64)),
    })
}

fn limit_clause(limit: Option<usize>) -> String {
    limit.map(|n| format!("LIMIT {n}")).unwrap_or_default()
}

fn group_artist(info: &TrackInfo) -> String {
    info.album_artist
        .clone()
        .or_else(|| info.artist.clone())
        .unwrap_or_default()
}

fn group_album(info: &TrackInfo) -> String {
    info.album.clone().unwrap_or_else(|| {
        info.path
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    })
}

/// Modification time in nanoseconds and size, to tell whether a file
/// changed since it was read.
fn stamp(path: &Path) -> Option<(i64, i64)> {
    let meta = fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some((mtime.as_nanos() as i64, meta.len() as i64))
}

/// Paths are stored as their bytes: file names need not be UTF-8.
fn bytes_path(bytes: &[u8]) -> PathBuf {
    PathBuf::from(OsString::from_vec(bytes.to_vec()))
}

fn escape_like(word: &str) -> String {
    let mut out = String::with_capacity(word.len());
    for c in word.chars() {
        if matches!(c, '%' | '_' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{TempDir, wav};
    use lofty::config::WriteOptions;
    use lofty::prelude::*;
    use lofty::tag::{Tag, TagType};

    fn formats() -> Formats {
        Formats {
            audio: vec!["wav".into()],
            image: vec!["jpg".into()],
            cover_names: vec!["cover".into()],
        }
    }

    fn song(path: &Path, artist: &str, album: &str, title: &str, n: u32, year: Option<&str>) {
        wav(path);
        let mut tag = Tag::new(TagType::Id3v2);
        tag.set_artist(artist.into());
        tag.set_album(album.into());
        tag.set_title(title.into());
        tag.set_track(n);
        if let Some(y) = year {
            tag.set_date(y.parse().unwrap());
        }
        tag.save_to_path(path, WriteOptions::default()).unwrap();
    }

    fn library(dir: &TempDir) -> Library {
        let a = dir.0.join("music/a");
        song(
            &a.join("2.wav"),
            "Aphex Twin",
            "Syro",
            "180db_",
            2,
            Some("2014"),
        );
        song(
            &a.join("1.wav"),
            "Aphex Twin",
            "Syro",
            "minipops 67",
            1,
            Some("2014"),
        );
        song(
            &a.join("saw.wav"),
            "Aphex Twin",
            "Selected Ambient Works 85–92",
            "Xtal",
            1,
            Some("1992"),
        );
        song(
            &dir.0.join("music/b/1.wav"),
            "Ёлка",
            "Альбом",
            "Прованс",
            1,
            None,
        );
        wav(&dir.0.join("music/Loose Folder/untagged.wav"));
        let mut lib = Library::in_memory().unwrap();
        lib.scan(&[dir.0.join("music")], &formats(), |_| {})
            .unwrap();
        lib
    }

    #[test]
    fn groups_tracks_into_artists_and_albums() {
        let dir = TempDir::new("lib-groups");
        let lib = library(&dir);
        assert_eq!(lib.track_count().unwrap(), 5);

        let artists = lib.artists().unwrap();
        let names: Vec<&str> = artists.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Aphex Twin", "Ёлка", ""]);
        assert_eq!(artists[0].albums, 2);

        let albums = lib.albums(Some("Aphex Twin")).unwrap();
        let titles: Vec<&str> = albums.iter().map(|a| a.title.as_str()).collect();
        assert_eq!(titles, ["Selected Ambient Works 85–92", "Syro"]);
        assert_eq!(albums[1].year, Some(2014));
        assert_eq!(albums[1].tracks, 2);
        assert_eq!(albums[1].first, dir.0.join("music/a/1.wav"));

        let tracks = lib.tracks(&albums[1]).unwrap();
        let titles: Vec<String> = tracks.iter().map(|t| t.display_title()).collect();
        assert_eq!(titles, ["minipops 67", "180db_"]);
        assert_eq!(tracks[0].duration, Some(Duration::from_secs(1)));

        // No tags: the folder is the album, the file name the title.
        let loose = &lib.albums(Some("")).unwrap()[0];
        assert_eq!(loose.title, "Loose Folder");
        assert_eq!(lib.tracks(loose).unwrap()[0].display_title(), "untagged");

        let all = lib.all_tracks().unwrap();
        assert_eq!(all.len(), 5);
        assert_eq!(all[0], dir.0.join("music/a/saw.wav"));
        assert_eq!(all[4], dir.0.join("music/Loose Folder/untagged.wav"));
    }

    #[test]
    fn search_matches_every_word_in_any_case() {
        let dir = TempDir::new("lib-search");
        let lib = library(&dir);

        let found = lib.search("aphex syro", 10).unwrap();
        assert!(found.artists.is_empty());
        assert_eq!(found.albums.len(), 1);
        assert_eq!(found.tracks.len(), 2);

        let found = lib.search("ЁЛКА", 10).unwrap();
        assert_eq!(found.artists[0].name, "Ёлка");
        assert_eq!(found.tracks[0].display_title(), "Прованс");

        // LIKE wildcards are plain characters.
        let found = lib.search("_", 10).unwrap();
        assert_eq!(found.tracks.len(), 1);
        assert_eq!(found.tracks[0].display_title(), "180db_");

        assert!(lib.search("  ", 10).unwrap().is_empty());
        assert_eq!(lib.search("a", 1).unwrap().tracks.len(), 1);
    }

    #[test]
    fn rescans_read_only_what_changed() {
        let dir = TempDir::new("lib-rescan");
        let mut lib = library(&dir);
        let root = [dir.0.join("music")];

        let mut reports = Vec::new();
        let stats = lib.scan(&root, &formats(), |p| reports.push(p)).unwrap();
        assert_eq!(stats, ScanStats::default());
        assert_eq!(reports, [Progress { done: 0, total: 0 }]);

        fs::remove_file(dir.0.join("music/b/1.wav")).unwrap();
        song(
            &dir.0.join("music/a/1.wav"),
            "Aphex Twin",
            "Syro",
            "Minipops 67 [120.2]",
            1,
            Some("2014"),
        );
        let stats = lib.scan(&root, &formats(), |_| {}).unwrap();
        assert_eq!(
            stats,
            ScanStats {
                read: 1,
                removed: 1
            }
        );
        let track = lib.track(&dir.0.join("music/a/1.wav")).unwrap().unwrap();
        assert_eq!(track.title.as_deref(), Some("Minipops 67 [120.2]"));
        assert_eq!(lib.artists().unwrap().len(), 2);
    }

    #[test]
    fn keeps_tracks_of_folders_that_are_not_there() {
        let dir = TempDir::new("lib-unmounted");
        let mut lib = library(&dir);
        let gone = dir.0.join("music");
        fs::rename(&gone, dir.0.join("elsewhere")).unwrap();
        let stats = lib.scan(&[gone], &formats(), |_| {}).unwrap();
        assert_eq!(stats.removed, 0);
        assert_eq!(lib.track_count().unwrap(), 5);
        // A folder that is no longer a root does drop its tracks.
        lib.scan(&[], &formats(), |_| {}).unwrap();
        assert_eq!(lib.track_count().unwrap(), 0);
    }

    #[test]
    fn library_file_survives_reopening() {
        let dir = TempDir::new("lib-file");
        let file = dir.0.join("data/library.db");
        song(&dir.0.join("music/1.wav"), "A", "B", "C", 1, None);
        {
            let mut lib = Library::open(&file).unwrap();
            lib.scan(&[dir.0.join("music")], &formats(), |_| {})
                .unwrap();
        }
        let lib = Library::open(&file).unwrap();
        assert_eq!(lib.track_count().unwrap(), 1);
    }
}
