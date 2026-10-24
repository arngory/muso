//! Files for tests: a folder that cleans up after itself and a tiny WAV.

use std::fs;
use std::path::{Path, PathBuf};

pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(name: &str) -> Self {
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

/// A second of 8 kHz mono silence, in a new folder if need be.
pub fn wav(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
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
