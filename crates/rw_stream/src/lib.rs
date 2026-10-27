//! Reader for the `RenderWare` binary stream.
//!
//! A `RenderWare` file — a DFF clump, a TXD dictionary, an IFP animation pack — is
//! a tree of sections. Each section starts with a 12-byte header: a type, the
//! payload length, and the library stamp of the tool that wrote it. Payloads of
//! container sections hold further sections; payloads of leaf sections are raw
//! data whose layout is defined by the format on top.
//!
//! Vice City mixes library stamps within one installation (DFF data is stamped
//! `0x0C02FFFF`, TXD data `0x1003FFFF`), so nothing here validates the stamp.
//! Callers that care read [`Header::stamp`] and branch themselves.
//!
//! # Examples
//!
//! ```
//! use rw_stream::{Section, SectionKind};
//!
//! // A clump holding one frame list, whose struct holds a zero frame count.
//! let bytes = [
//!     0x10, 0x00, 0x00, 0x00, 28, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0x02, 0x0C, // clump
//!     0x0E, 0x00, 0x00, 0x00, 16, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0x02, 0x0C, // frame list
//!     0x01, 0x00, 0x00, 0x00, 4, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0x02, 0x0C, // struct
//!     0x00, 0x00, 0x00, 0x00, // zero frames
//! ];
//!
//! let clump = Section::root(&bytes)?;
//! assert_eq!(clump.kind(), SectionKind::CLUMP);
//!
//! let frames = clump.child(SectionKind::FRAME_LIST)?.expect("frame list");
//! let count = frames.child(SectionKind::STRUCT)?.expect("struct").u32_at(0)?;
//! assert_eq!(count, 0);
//! # Ok::<(), rw_stream::StreamError>(())
//! ```

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

use thiserror::Error;

/// Size of a section header in bytes.
pub const HEADER_SIZE: usize = 12;

/// Section type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SectionKind(pub u32);

impl SectionKind {
    /// Struct: a fixed-layout data blob, not a container.
    pub const STRUCT: Self = Self(0x01);
    /// String: a counted or NUL-terminated string, not a container.
    pub const STRING: Self = Self(0x02);
    /// Extension, the parent of vendor-specific children.
    pub const EXTENSION: Self = Self(0x03);
    /// Camera.
    pub const CAMERA: Self = Self(0x05);
    /// Material.
    pub const MATERIAL: Self = Self(0x07);
    /// Material list.
    pub const MATERIAL_LIST: Self = Self(0x08);
    /// Frame list.
    pub const FRAME_LIST: Self = Self(0x0E);
    /// Geometry.
    pub const GEOMETRY: Self = Self(0x0F);
    /// Clump: the top level section of a DFF.
    pub const CLUMP: Self = Self(0x10);
    /// Atomic: a frame paired with a geometry.
    pub const ATOMIC: Self = Self(0x14);
    /// Texture native: one texture inside a dictionary.
    pub const TEXTURE_NATIVE: Self = Self(0x15);
    /// Texture dictionary: the top level section of a TXD.
    pub const TEXTURE_DICTIONARY: Self = Self(0x16);
    /// Geometry list.
    pub const GEOMETRY_LIST: Self = Self(0x1A);
    /// Skin.
    pub const SKIN: Self = Self(0x0110);

    /// Whether this kind's payload holds further sections.
    ///
    /// Derived from Vice City's own files: `STRUCT` and `STRING` payloads do not
    /// nest, and neither do the frame-tag sections found under `EXTENSION`.
    pub const fn is_container(self) -> bool {
        matches!(
            self.0,
            0x03 | 0x05
                | 0x07
                | 0x08
                | 0x09
                | 0x0E
                | 0x0F
                | 0x10
                | 0x14
                | 0x15
                | 0x16
                | 0x1A
                | 0x1B
                | 0x120
        )
    }
}

/// Library stamp of a section, for example `0x0C02FFFF` for Vice City DFF data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LibraryStamp(pub u32);

impl std::fmt::Display for LibraryStamp {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "0x{:08X}", self.0)
    }
}

impl std::fmt::Display for SectionKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "0x{:02X}", self.0)
    }
}

/// A section header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// Section type.
    pub kind: SectionKind,
    /// Payload length in bytes, excluding the header.
    pub size: u32,
    /// Library stamp of the tool that wrote the section.
    pub stamp: LibraryStamp,
}

/// A section: its header and its payload.
#[derive(Debug, Clone, Copy)]
pub struct Section<'a> {
    header: Header,
    payload: &'a [u8],
}

impl<'a> Section<'a> {
    /// Reads the single top level section of `data`.
    ///
    /// # Errors
    /// Returns a [`StreamError`] when the first header is truncated or its
    /// payload is longer than the remaining data.
    pub fn root(data: &'a [u8]) -> Result<Self, StreamError> {
        Sections::new(data)
            .next()
            .transpose()?
            .ok_or(StreamError::Empty)
    }

    /// Reads the section that starts at `offset`.
    ///
    /// # Errors
    /// Returns a [`StreamError`] when the header is truncated or the payload
    /// runs past the end of `data`.
    pub fn at(data: &'a [u8], offset: usize) -> Result<Self, StreamError> {
        let rest = data
            .get(offset..)
            .ok_or(StreamError::TruncatedHeader { offset })?;

        if rest.len() < HEADER_SIZE {
            return Err(StreamError::TruncatedHeader { offset });
        }

        let size = u32::from_le_bytes([rest[4], rest[5], rest[6], rest[7]]) as usize;
        let payload =
            rest.get(HEADER_SIZE..HEADER_SIZE + size)
                .ok_or(StreamError::SectionOverrun {
                    offset,
                    size,
                    available: rest.len() - HEADER_SIZE,
                })?;

        Ok(Self {
            header: Header {
                kind: SectionKind(u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]])),
                size: size as u32,
                stamp: LibraryStamp(u32::from_le_bytes([rest[8], rest[9], rest[10], rest[11]])),
            },
            payload,
        })
    }

    /// This section's header.
    pub const fn header(&self) -> Header {
        self.header
    }

    /// This section's type.
    pub const fn kind(&self) -> SectionKind {
        self.header.kind
    }

    /// Payload length in bytes, excluding the header.
    pub const fn size(&self) -> u32 {
        self.header.size
    }

    /// Library stamp of the tool that wrote this section.
    pub const fn stamp(&self) -> LibraryStamp {
        self.header.stamp
    }

    /// The section payload.
    pub const fn payload(&self) -> &'a [u8] {
        self.payload
    }

    /// The child sections of this section's payload.
    ///
    /// # Errors
    /// Returns [`StreamError::NotAContainer`] for leaf kinds such as
    /// [`SectionKind::STRUCT`].
    pub const fn children(&self) -> Result<Sections<'a>, StreamError> {
        if !self.header.kind.is_container() {
            return Err(StreamError::NotAContainer {
                kind: self.header.kind,
            });
        }

        Ok(Sections::new(self.payload))
    }

    /// The first child of `kind`, if the section has one.
    ///
    /// # Errors
    /// Propagates [`StreamError`] from walking the child sections.
    pub fn child(&self, kind: SectionKind) -> Result<Option<Self>, StreamError> {
        for child in self.children()? {
            let child = child?;

            if child.kind() == kind {
                return Ok(Some(child));
            }
        }

        Ok(None)
    }

    /// Every child of `kind`.
    ///
    /// # Errors
    /// Propagates [`StreamError`] from walking the child sections.
    pub fn children_of(&self, kind: SectionKind) -> Result<Vec<Self>, StreamError> {
        let mut found = Vec::new();

        for child in self.children()? {
            let child = child?;

            if child.kind() == kind {
                found.push(child);
            }
        }

        Ok(found)
    }

    /// Reads a little-endian `u32` from the payload.
    ///
    /// # Errors
    /// Returns [`StreamError::PayloadTooShort`] when the payload does not hold
    /// four bytes at `offset`.
    pub fn u32_at(&self, offset: usize) -> Result<u32, StreamError> {
        let bytes = self
            .payload
            .get(offset..offset + 4)
            .ok_or(StreamError::PayloadTooShort {
                offset,
                len: self.payload.len(),
            })?;

        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// Reads the two little-endian `u16`s at `offset`.
    ///
    /// # Errors
    /// Returns [`StreamError::PayloadTooShort`] when the payload does not hold
    /// four bytes at `offset`.
    pub fn u16x2_at(&self, offset: usize) -> Result<(u16, u16), StreamError> {
        let bytes = self
            .payload
            .get(offset..offset + 4)
            .ok_or(StreamError::PayloadTooShort {
                offset,
                len: self.payload.len(),
            })?;

        Ok((
            u16::from_le_bytes([bytes[0], bytes[1]]),
            u16::from_le_bytes([bytes[2], bytes[3]]),
        ))
    }

    /// Reads a NUL-terminated string, lossily, capped at `width` bytes.
    pub fn string_at(&self, offset: usize, width: usize) -> Option<String> {
        let bytes = self.payload.get(offset..offset + width)?;
        let end = bytes.iter().position(|byte| *byte == 0).unwrap_or(width);

        Some(String::from_utf8_lossy(&bytes[..end]).into_owned())
    }
}

/// Iterator over the sibling sections stored in a byte range.
///
/// Iteration is flat: one level only. Callers that want a tree recurse through
/// [`Section::children`], which is why there is no nesting depth limit here.
#[derive(Debug, Clone)]
pub struct Sections<'a> {
    data: &'a [u8],
    offset: usize,
}

impl<'a> Sections<'a> {
    /// Iterates the sections packed into `data`.
    pub const fn new(data: &'a [u8]) -> Self {
        Self { data, offset: 0 }
    }
}

impl<'a> Iterator for Sections<'a> {
    type Item = Result<Section<'a>, StreamError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.offset >= self.data.len() {
            return None;
        }

        let section = match Section::at(self.data, self.offset) {
            Ok(section) => section,
            Err(error) => {
                self.offset = self.data.len();

                return Some(Err(error));
            }
        };

        self.offset += HEADER_SIZE + section.size() as usize;

        Some(Ok(section))
    }
}

/// Failures while reading a `RenderWare` stream.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum StreamError {
    /// The input has no sections at all.
    #[error("no section found")]
    Empty,
    /// A section header runs past the end of the input.
    #[error("truncated section header at offset {offset}")]
    TruncatedHeader {
        /// Byte offset of the header.
        offset: usize,
    },
    /// A section claims more payload than remains.
    #[error("section at offset {offset} claims {size} bytes, only {available} remain")]
    SectionOverrun {
        /// Byte offset of the header.
        offset: usize,
        /// Claimed payload length.
        size: usize,
        /// Bytes actually available after the header.
        available: usize,
    },
    /// Child sections were requested from a leaf section.
    #[error("section of kind {kind} holds no child sections")]
    NotAContainer {
        /// The offending section kind.
        kind: SectionKind,
    },
    /// A payload field was read beyond the end of the payload.
    #[error("payload of {len} bytes has no u32 at offset {offset}")]
    PayloadTooShort {
        /// Requested offset.
        offset: usize,
        /// Payload length.
        len: usize,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn header(kind: u32, size: u32, stamp: u32) -> [u8; HEADER_SIZE] {
        let mut bytes = [0u8; HEADER_SIZE];
        bytes[0..4].copy_from_slice(&kind.to_le_bytes());
        bytes[4..8].copy_from_slice(&size.to_le_bytes());
        bytes[8..12].copy_from_slice(&stamp.to_le_bytes());
        bytes
    }

    #[test]
    fn reads_the_single_root_section() {
        let mut data = header(0x10, 4, 0x0C02_FFFF).to_vec();
        data.extend([0u8; 4]);

        let root = Section::root(&data).unwrap();

        assert_eq!(root.kind(), SectionKind::CLUMP);
        assert_eq!(root.size(), 4);
    }

    #[test]
    fn rejects_an_empty_input() {
        let result = Section::root(&[]);

        assert_eq!(result.unwrap_err(), StreamError::Empty);
    }

    #[test]
    fn rejects_a_payload_that_runs_past_the_input() {
        let data = header(0x10, 4096, 0);

        let result = Section::root(&data);

        assert!(matches!(
            result,
            Err(StreamError::SectionOverrun { size: 4096, .. })
        ));
    }

    #[test]
    fn rejects_children_of_a_struct() {
        let data = header(0x01, 0, 0).to_vec();

        let section = Section::at(&data, 0).unwrap();
        let result = section.children();

        assert!(matches!(result, Err(StreamError::NotAContainer { .. })));
    }

    #[test]
    fn iterates_sibling_sections() {
        let mut data = header(0x0F, 0, 0).to_vec();
        data.extend(header(0x0F, 0, 0));

        let sections: Vec<_> = Sections::new(&data).map(Result::unwrap).collect();

        assert_eq!(sections.len(), 2);
        assert_eq!(sections[1].kind(), SectionKind::GEOMETRY);
    }

    #[test]
    fn iterates_the_children_of_a_container() {
        let mut data = header(0x1A, 12, 0).to_vec();
        data.extend(header(0x0F, 0, 0));

        let container = Section::root(&data).unwrap();
        let children: Vec<_> = container.children().unwrap().map(Result::unwrap).collect();

        assert_eq!(children.len(), 1);
        assert_eq!(children[0].kind(), SectionKind::GEOMETRY);
    }

    #[test]
    fn reads_a_u32_from_the_payload() {
        let mut data = header(0x01, 4, 0).to_vec();
        data.extend(0x2710u32.to_le_bytes());

        let section = Section::at(&data, 0).unwrap();

        assert_eq!(section.u32_at(0).unwrap(), 10_000);
    }

    #[test]
    fn rejects_a_u32_read_past_the_payload() {
        let mut data = header(0x01, 2, 0).to_vec();
        data.extend([0u8; 2]);

        let result = Section::at(&data, 0).unwrap().u32_at(0);

        assert!(matches!(result, Err(StreamError::PayloadTooShort { .. })));
    }

    #[test]
    fn reads_a_padded_name() {
        let mut payload = vec![0u8; 32];
        payload[..7].copy_from_slice(b"admiral");
        let mut data = header(0x01, 32, 0).to_vec();
        data.extend(payload);

        let section = Section::at(&data, 0).unwrap();

        assert_eq!(section.string_at(0, 32).as_deref(), Some("admiral"));
    }
}
