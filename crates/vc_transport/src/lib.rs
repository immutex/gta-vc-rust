//! Locates a Grand Theft Auto: Vice City installation.
//!
//! This crate resolves the installation root, validates that it looks like Vice
//! City 1.0, and maps the IMG archive read-only. It does not interpret the
//! formats inside the archive: those have a crate each.
//!
//! # Examples
//!
//! ```no_run
//! use vc_transport::GameRoot;
//!
//! let root = GameRoot::from_env()?;
//! println!("using {}", root.path().display());
//! # Ok::<(), vc_transport::TransportError>(())
//! ```

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

pub mod img;

use std::env;
use std::path::{Path, PathBuf};

use thiserror::Error;

pub use img::{ImageArchive, ImgEntry};

/// Environment variable holding the installation root.
pub const GAME_ROOT_ENV: &str = "VC_GAME_ROOT";

/// Path used when [`GAME_ROOT_ENV`] is not set.
pub const DEFAULT_GAME_ROOT: &str = "gta-vc";

/// Relative path of the main asset archive.
const IMAGE_FILE: &str = "models/gta3.img";

/// A validated Vice City installation root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameRoot {
    path: PathBuf,
}

impl GameRoot {
    /// Resolves the installation root from [`GAME_ROOT_ENV`], falling back to
    /// [`DEFAULT_GAME_ROOT`].
    ///
    /// # Errors
    /// Returns [`TransportError::NotAnInstall`] when the resolved directory is
    /// not a Vice City installation.
    pub fn from_env() -> Result<Self, TransportError> {
        let path = env::var_os(GAME_ROOT_ENV)
            .map_or_else(|| PathBuf::from(DEFAULT_GAME_ROOT), PathBuf::from);

        Self::new(path)
    }

    /// Validates `path` as an installation root.
    ///
    /// # Errors
    /// Returns [`TransportError::NotAnInstall`] when `models/gta3.img` does not
    /// exist under `path`.
    pub fn new(path: impl Into<PathBuf>) -> Result<Self, TransportError> {
        let path = path.into();

        if !path.join(IMAGE_FILE).is_file() {
            return Err(TransportError::NotAnInstall { path });
        }

        Ok(Self { path })
    }

    /// The installation directory.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Path of the main asset archive, `models/gta3.img`.
    pub fn image(&self) -> PathBuf {
        self.path.join(IMAGE_FILE)
    }
}

/// Failures while locating an installation or reading its archives.
#[derive(Debug, Error)]
pub enum TransportError {
    /// The directory is missing the files a Vice City installation must have.
    #[error("`{}` is not a Vice City install: {IMAGE_FILE} is missing", path.display())]
    NotAnInstall {
        /// The rejected path.
        path: PathBuf,
    },
    /// An IMG archive has no sibling directory beside it.
    #[error("`{}` is missing; an IMG archive needs its sibling directory", path.display())]
    MissingDirectory {
        /// Expected directory path.
        path: PathBuf,
    },
    /// A file could not be read.
    #[error("failed to read `{}`", path.display())]
    Io {
        /// The offending path.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: std::io::Error,
    },
    /// An entry index is past the end of the directory.
    #[error("entry index {index} is out of range: the directory holds {entries} entries")]
    NoSuchEntry {
        /// Requested index.
        index: usize,
        /// Number of entries.
        entries: usize,
    },
    /// An entry points past the end of the image.
    #[error("entry {index} (`{name}`) points outside the image")]
    EntryOutOfRange {
        /// Entry index.
        index: usize,
        /// Entry name.
        name: String,
    },
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        env::temp_dir().join(format!("vc-rs-transport-{name}"))
    }

    #[test]
    fn rejects_directory_without_image() {
        let dir = temp_root("no-image");
        fs::create_dir_all(&dir).unwrap();

        let result = GameRoot::new(&dir);

        assert!(
            matches!(result, Err(TransportError::NotAnInstall { .. })),
            "expected NotAnInstall, got {result:?}"
        );
    }

    #[test]
    fn accepts_directory_with_image() {
        let dir = temp_root("with-image");
        fs::create_dir_all(dir.join("models")).unwrap();
        fs::write(dir.join(IMAGE_FILE), b"").unwrap();

        let root = GameRoot::new(&dir).unwrap();

        assert_eq!(root.path(), dir);
    }
}
