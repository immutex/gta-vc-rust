//! Summaries of Vice City DFF clumps.
//!
//! A clump is a frame hierarchy, a list of geometries and one atomic per drawn
//! part. Each geometry opens with a struct that states its flags and its
//! triangle, vertex and morph target counts, which is enough to size the whole
//! model set without decoding vertex data.
//!
//! # Examples
//!
//! ```
//! use vc_dff::summarize;
//! use rw_stream::{SectionKind, HEADER_SIZE};
//!
//! fn section(kind: u32, payload: &[u8]) -> Vec<u8> {
//!     let mut bytes = kind.to_le_bytes().to_vec();
//!     bytes.extend((payload.len() as u32).to_le_bytes());
//!     bytes.extend(0x0C02_FFFFu32.to_le_bytes());
//!     bytes.extend(payload);
//!     bytes
//! }
//!
//! let geometry = section(
//!     SectionKind::GEOMETRY.0,
//!     &section(SectionKind::STRUCT.0, &{
//!         let mut struct_payload = 0x10077u32.to_le_bytes().to_vec();
//!         struct_payload.extend(106u32.to_le_bytes()); // triangles
//!         struct_payload.extend(184u32.to_le_bytes()); // vertices
//!         struct_payload.extend(1u32.to_le_bytes()); // morph targets
//!         struct_payload
//!     }),
//! );
//! let mut list = section(SectionKind::STRUCT.0, &1u32.to_le_bytes()); // declared count
//! list.extend(geometry);
//! let clump = section(SectionKind::CLUMP.0, &section(SectionKind::GEOMETRY_LIST.0, &list));
//!
//! let summary = summarize(&clump)?;
//! assert_eq!(summary.triangles, 106);
//! assert_eq!(summary.geometries, 1);
//! # Ok::<(), vc_dff::DffError>(())
//! ```

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

use rw_stream::{Section, SectionKind, StreamError};
use thiserror::Error;

/// Offsets inside a geometry struct, which states counts as four `u32`s.
const GEOMETRY_TRIANGLES: usize = 4;
const GEOMETRY_VERTICES: usize = 8;
const GEOMETRY_MORPH_TARGETS: usize = 12;

/// Contents of one clump.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Clump {
    /// Atomically drawn parts.
    pub atomics: u32,
    /// Frames in the hierarchy.
    pub frames: u32,
    /// Geometries found.
    pub geometries: u32,
    /// Geometries the geometry list declares, for cross-checking.
    pub declared_geometries: u32,
    /// Triangles across all geometries.
    pub triangles: u64,
    /// Vertices across all geometries.
    pub vertices: u64,
    /// Morph targets across all geometries.
    pub morph_targets: u64,
    /// Materials across all geometry material lists.
    pub materials: u32,
}

impl Clump {
    /// Whether the declared geometry count matches the geometries found.
    pub const fn geometry_count_matches(&self) -> bool {
        self.declared_geometries == self.geometries
    }
}

/// Summarises the clump in `bytes`.
///
/// # Errors
/// Returns [`DffError::UnexpectedRoot`] when the file is not a clump,
/// [`DffError::MissingStruct`] when a section that must open with a struct does
/// not, and propagates [`StreamError`] for malformed section trees.
pub fn summarize(bytes: &[u8]) -> Result<Clump, DffError> {
    let root = Section::root(bytes)?;

    if root.kind() != SectionKind::CLUMP {
        return Err(DffError::UnexpectedRoot { kind: root.kind() });
    }

    let mut clump = Clump {
        atomics: root.children_of(SectionKind::ATOMIC)?.len() as u32,
        ..Clump::default()
    };

    if let Some(frame_list) = root.child(SectionKind::FRAME_LIST)? {
        let frame_struct =
            frame_list
                .child(SectionKind::STRUCT)?
                .ok_or(DffError::MissingStruct {
                    parent: SectionKind::FRAME_LIST,
                })?;

        clump.frames = frame_struct.u32_at(0)?;
    }

    for geometry_list in root.children_of(SectionKind::GEOMETRY_LIST)? {
        let list_struct =
            geometry_list
                .child(SectionKind::STRUCT)?
                .ok_or(DffError::MissingStruct {
                    parent: SectionKind::GEOMETRY_LIST,
                })?;

        clump.declared_geometries += list_struct.u32_at(0)?;

        for geometry in geometry_list.children_of(SectionKind::GEOMETRY)? {
            let geometry_struct =
                geometry
                    .child(SectionKind::STRUCT)?
                    .ok_or(DffError::MissingStruct {
                        parent: SectionKind::GEOMETRY,
                    })?;

            clump.geometries += 1;
            clump.triangles += u64::from(geometry_struct.u32_at(GEOMETRY_TRIANGLES)?);
            clump.vertices += u64::from(geometry_struct.u32_at(GEOMETRY_VERTICES)?);
            clump.morph_targets += u64::from(geometry_struct.u32_at(GEOMETRY_MORPH_TARGETS)?);

            if let Some(material_list) = geometry.child(SectionKind::MATERIAL_LIST)? {
                clump.materials += material_list.children_of(SectionKind::MATERIAL)?.len() as u32;
            }
        }
    }

    Ok(clump)
}

/// Failures while summarising a clump.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DffError {
    /// The file's top level section is not a clump.
    #[error("expected a clump, found section kind {kind}")]
    UnexpectedRoot {
        /// The kind that was found.
        kind: SectionKind,
    },
    /// A section that must open with a struct does not.
    #[error("section {parent} has no struct child")]
    MissingStruct {
        /// The parent section kind.
        parent: SectionKind,
    },
    /// The section tree is malformed.
    #[error(transparent)]
    Stream(#[from] StreamError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use rw_stream::HEADER_SIZE;

    fn section(kind: SectionKind, payload: &[u8]) -> Vec<u8> {
        let mut bytes = kind.0.to_le_bytes().to_vec();
        bytes.extend((payload.len() as u32).to_le_bytes());
        bytes.extend(0x0C02_FFFFu32.to_le_bytes());
        bytes.extend(payload);

        bytes
    }

    fn geometry_struct(triangles: u32, vertices: u32, morph_targets: u32) -> Vec<u8> {
        let mut payload = 0x0001_0077u32.to_le_bytes().to_vec();
        payload.extend(triangles.to_le_bytes());
        payload.extend(vertices.to_le_bytes());
        payload.extend(morph_targets.to_le_bytes());

        section(SectionKind::STRUCT, &payload)
    }

    fn clump_with(geometries: &[u32]) -> Vec<u8> {
        let mut body = section(
            SectionKind::STRUCT,
            &(geometries.len() as u32).to_le_bytes(),
        );

        for triangles in geometries {
            let mut geometry = geometry_struct(*triangles, 10, 1);
            let mut materials = section(SectionKind::MATERIAL, &[0u8; 4]);
            materials.extend(section(SectionKind::MATERIAL, &[0u8; 4]));
            geometry.extend(section(SectionKind::MATERIAL_LIST, &materials));
            body.extend(section(SectionKind::GEOMETRY, &geometry));
        }

        section(
            SectionKind::CLUMP,
            &section(SectionKind::GEOMETRY_LIST, &body),
        )
    }

    #[test]
    fn sums_triangles_across_geometries() {
        let bytes = clump_with(&[106, 98, 728]);

        let clump = summarize(&bytes).unwrap();

        assert_eq!(clump.triangles, 932);
    }

    #[test]
    fn counts_geometries_and_materials() {
        let bytes = clump_with(&[10, 20]);

        let clump = summarize(&bytes).unwrap();

        assert_eq!(clump.geometries, 2);
        assert_eq!(clump.materials, 4);
    }

    #[test]
    fn reports_when_the_declared_geometry_count_differs() {
        let mut body = section(SectionKind::STRUCT, &7u32.to_le_bytes());
        body.extend(section(SectionKind::GEOMETRY, &geometry_struct(1, 1, 1)));
        let bytes = section(
            SectionKind::CLUMP,
            &section(SectionKind::GEOMETRY_LIST, &body),
        );

        let clump = summarize(&bytes).unwrap();

        assert!(!clump.geometry_count_matches());
    }

    #[test]
    fn rejects_a_file_that_is_not_a_clump() {
        let bytes = section(SectionKind::TEXTURE_DICTIONARY, &[]);

        let result = summarize(&bytes);

        assert!(matches!(result, Err(DffError::UnexpectedRoot { .. })));
    }

    #[test]
    fn rejects_a_geometry_list_without_a_struct() {
        let list = section(SectionKind::GEOMETRY, &geometry_struct(1, 1, 1));
        let bytes = section(
            SectionKind::CLUMP,
            &section(SectionKind::GEOMETRY_LIST, &list),
        );

        let result = summarize(&bytes);

        assert!(matches!(
            result,
            Err(DffError::MissingStruct {
                parent: SectionKind::GEOMETRY_LIST
            })
        ));
    }

    #[test]
    fn counts_frames_from_the_frame_list() {
        let mut clump = section(
            SectionKind::FRAME_LIST,
            &section(SectionKind::STRUCT, &5u32.to_le_bytes()),
        );
        clump.extend(section(
            SectionKind::GEOMETRY_LIST,
            &section(SectionKind::STRUCT, &0u32.to_le_bytes()),
        ));
        let bytes = section(SectionKind::CLUMP, &clump);

        let frames = summarize(&bytes).unwrap().frames;

        assert_eq!(frames, 5);
    }

    #[test]
    fn keeps_the_header_size_consistent_with_the_reader() {
        assert_eq!(HEADER_SIZE, 12);
    }
}
