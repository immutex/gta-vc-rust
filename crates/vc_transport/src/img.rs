//! Read-only access to the IMG version 1 archive.
//!
//! The archive is split in two files: `.dir` holds a headerless directory of
//! 32-byte entries and `.img` holds the payloads. Offsets and lengths are
//! counted in 2,048-byte sectors. Names are not unique — Vice City ships
//! several duplicate entries — so callers address files by index and never
//! assume a name identifies one file.

use std::fs;
use std::path::{Path, PathBuf};

use memmap2::Mmap;

use crate::TransportError;

/// Size of one directory entry.
pub const DIR_ENTRY_SIZE: usize = 32;

/// Sector size that offsets and lengths are counted in.
pub const SECTOR_SIZE: u64 = 2048;

/// Width of the name field inside a directory entry.
pub const NAME_WIDTH: usize = 24;

/// One file inside the archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImgEntry {
    name: String,
    sector_offset: u32,
    sector_len: u32,
}

impl ImgEntry {
    /// Name as stored in the directory.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Byte offset of the payload inside the image.
    pub fn byte_offset(&self) -> u64 {
        u64::from(self.sector_offset) * SECTOR_SIZE
    }

    /// Byte length of the payload.
    pub fn byte_len(&self) -> u64 {
        u64::from(self.sector_len) * SECTOR_SIZE
    }

    /// Whether the entry occupies no sectors.
    pub const fn is_empty(&self) -> bool {
        self.sector_len == 0
    }

    /// Whether the name ends with `extension`, ignoring case.
    pub fn has_extension(&self, extension: &str) -> bool {
        self.name
            .rsplit_once('.')
            .is_some_and(|(_, suffix)| suffix.eq_ignore_ascii_case(extension))
    }
}

/// A memory-mapped image together with the directory that describes it.
#[derive(Debug)]
pub struct ImageArchive {
    image: Mmap,
    path: PathBuf,
    entries: Vec<ImgEntry>,
}

impl ImageArchive {
    /// Opens an image and the directory beside it.
    ///
    /// # Errors
    /// Returns [`TransportError::MissingDirectory`] when the sibling `.dir` file
    /// does not exist, [`TransportError::Io`] when either file cannot be read,
    /// and [`TransportError::EntryOutOfRange`] when an entry points outside the
    /// image.
    pub fn open(image: &Path) -> Result<Self, TransportError> {
        let directory_path = image.with_extension("dir");

        if !directory_path.is_file() {
            return Err(TransportError::MissingDirectory {
                path: directory_path,
            });
        }

        let directory = fs::read(&directory_path).map_err(|source| TransportError::Io {
            path: directory_path,
            source,
        })?;

        let file = fs::File::open(image).map_err(|source| TransportError::Io {
            path: image.to_path_buf(),
            source,
        })?;

        let mapped = map_read_only(&file, image)?;

        let entries = parse_directory(&directory);
        let archive = Self {
            image: mapped,
            path: image.to_path_buf(),
            entries,
        };

        archive.validate()?;

        Ok(archive)
    }

    /// Path of the image file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Every entry, in directory order.
    pub fn entries(&self) -> &[ImgEntry] {
        &self.entries
    }

    /// Number of entries.
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the directory is empty.
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Payload of the entry at `index`.
    ///
    /// # Errors
    /// Returns [`TransportError::EntryOutOfRange`] when `index` is not an entry.
    pub fn data(&self, index: usize) -> Result<&[u8], TransportError> {
        let entry_count = self.entries.len();
        let entry = self.entries.get(index).ok_or(TransportError::NoSuchEntry {
            index,
            entries: entry_count,
        })?;

        let start = entry.byte_offset() as usize;
        let end = start + entry.byte_len() as usize;
        let name = entry.name.clone();

        self.image
            .get(start..end)
            .ok_or(TransportError::EntryOutOfRange { index, name })
    }

    /// Names that appear more than once, sorted and deduplicated.
    pub fn duplicate_names(&self) -> Vec<String> {
        duplicate_names(&self.entries)
    }

    /// Index of every entry whose name matches, ignoring case.
    pub fn indices_of(&self, name: &str) -> Vec<usize> {
        indices_of(&self.entries, name)
    }

    fn validate(&self) -> Result<(), TransportError> {
        let image_len = self.image.len() as u64;

        for (index, entry) in self.entries.iter().enumerate() {
            if entry.byte_offset() + entry.byte_len() > image_len {
                return Err(TransportError::EntryOutOfRange {
                    index,
                    name: entry.name.clone(),
                });
            }
        }

        Ok(())
    }
}

fn map_read_only(file: &fs::File, path: &Path) -> Result<Mmap, TransportError> {
    // SAFETY: the archive is opened read-only and this process never writes to
    // it. Another process modifying the mapped file would be undefined, which is
    // the documented contract of memmap2.
    #[expect(unsafe_code, reason = "memmap2 has no safe constructor for a mapping")]
    let mapped = unsafe { Mmap::map(file) };

    mapped.map_err(|source| TransportError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn duplicate_names(entries: &[ImgEntry]) -> Vec<String> {
    let mut seen: Vec<&str> = Vec::new();
    let mut duplicates: Vec<&str> = Vec::new();

    for entry in entries {
        let name = entry.name.as_str();

        if seen.contains(&name) {
            if !duplicates.contains(&name) {
                duplicates.push(name);
            }
        } else {
            seen.push(name);
        }
    }

    duplicates.sort_unstable();
    duplicates.into_iter().map(str::to_owned).collect()
}

fn indices_of(entries: &[ImgEntry], name: &str) -> Vec<usize> {
    entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| entry.name.eq_ignore_ascii_case(name))
        .map(|(index, _)| index)
        .collect()
}

fn parse_directory(directory: &[u8]) -> Vec<ImgEntry> {
    directory
        .as_chunks::<DIR_ENTRY_SIZE>()
        .0
        .iter()
        .map(|entry| {
            let sector_offset = u32::from_le_bytes([entry[0], entry[1], entry[2], entry[3]]);
            let sector_len = u32::from_le_bytes([entry[4], entry[5], entry[6], entry[7]]);

            ImgEntry {
                name: read_name(&entry[8..8 + NAME_WIDTH]),
                sector_offset,
                sector_len,
            }
        })
        .collect()
}

fn read_name(field: &[u8]) -> String {
    field
        .iter()
        .take_while(|byte| **byte != 0)
        .map(|byte| char::from(*byte))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn directory_entry(name: &str, sector_offset: u32, sector_len: u32) -> Vec<u8> {
        let mut entry = sector_offset.to_le_bytes().to_vec();
        entry.extend(sector_len.to_le_bytes());
        entry.extend(name.as_bytes());
        entry.resize(DIR_ENTRY_SIZE, 0);

        entry
    }

    #[test]
    fn reads_a_directory_entry() {
        let entries = parse_directory(&directory_entry("radar00.txd", 5, 2));

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name(), "radar00.txd");
    }

    #[test]
    fn converts_sectors_to_bytes() {
        let entries = parse_directory(&directory_entry("radar00.txd", 5, 2));

        assert_eq!(entries[0].byte_offset(), 10_240);
        assert_eq!(entries[0].byte_len(), 4_096);
    }

    #[test]
    fn ignores_trailing_bytes_that_do_not_fill_an_entry() {
        let mut directory = directory_entry("radar00.txd", 0, 1);
        directory.extend([0u8; 7]);

        let entries = parse_directory(&directory);

        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn matches_extensions_ignoring_case() {
        let entries = parse_directory(&directory_entry("SPLASH1.TXD", 0, 1));

        assert!(entries[0].has_extension("txd"));
    }

    #[test]
    fn detects_a_duplicate_name() {
        let mut directory = directory_entry("generic.txd", 0, 1);
        directory.extend(directory_entry("generic.txd", 1, 1));
        directory.extend(directory_entry("chef.dff", 2, 1));

        let duplicates = duplicate_names(&parse_directory(&directory));

        assert_eq!(duplicates, vec!["generic.txd".to_owned()]);
    }

    #[test]
    fn finds_every_index_of_a_name() {
        let mut directory = directory_entry("chef.dff", 0, 1);
        directory.extend(directory_entry("chef.dff", 1, 1));

        let entries = parse_directory(&directory);
        let mut archive = Vec::new();
        archive.extend(entries);

        let indices: Vec<usize> = archive
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.name == "chef.dff")
            .map(|(index, _)| index)
            .collect();

        assert_eq!(indices, vec![0, 1]);
    }
}
