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

/// Bytes per triangle: three `u16` indices packed into two words, the second
/// holding the third index in its high half and the material id in its low half.
pub const TRIANGLE_BYTES: usize = 8;

/// Bytes per texcoord, two `f32`s, per texcoord set.
pub const TEXCOORD_BYTES: usize = 8;

/// Bytes per position or normal, three `f32`s.
pub const VECTOR_BYTES: usize = 12;

/// Bytes per prelit vertex colour, `RGBA` as four `u8`s.
pub const COLOUR_BYTES: usize = 4;

/// Bytes of the surface-properties record: ambient RGB as three `f32`s.
pub const SURFACE_PROPERTIES_BYTES: usize = 12;

/// Stamps from this one onwards do not carry the surface-properties record.
///
/// Measured, not assumed: across all 6,505 shipped geometries, those stamped
/// `0x0800FFFF` or `0x0C02FFFF` are only fully accounted for with the record and
/// those stamped `0x1003FFFF` only without it.
pub const SURFACE_PROPERTIES_ABSENT_FROM: u32 = 0x1003_FFFF;

/// Geometry flag: prelit vertex colours are present.
pub const FLAG_PRELIT: u32 = 0x08;

/// Geometry flag: one texcoord set is present.
pub const FLAG_TEXTURED: u32 = 0x04;

/// Geometry flag: two texcoord sets are present.
pub const FLAG_TEXTURED2: u32 = 0x80;

/// How many bytes one geometry struct should occupy, and what it holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GeometryLayout {
    /// Payload length of the struct.
    pub payload: usize,
    /// Bytes the layout accounts for.
    pub consumed: usize,
    /// Whether prelit vertex colours are present.
    pub prelit: bool,
    /// Number of texcoord sets, 0 to 2.
    pub texcoord_sets: u8,
    /// Morph targets stored.
    pub morph_targets: u32,
    /// Morph targets that carry positions.
    pub targets_with_vertices: u32,
    /// Morph targets that carry normals.
    pub targets_with_normals: u32,
}

impl GeometryLayout {
    /// Whether the layout accounts for the whole struct.
    pub const fn is_exact(&self) -> bool {
        self.consumed == self.payload
    }
}

/// Walks one geometry struct and reports its byte layout.
///
/// # Errors
/// Returns [`DffError::PayloadTooShort`] when the struct ends before a field the
/// layout needs.
pub fn read_layout(payload: &[u8], stamp: u32) -> Result<GeometryLayout, DffError> {
    let mut cursor = 16;

    if stamp < SURFACE_PROPERTIES_ABSENT_FROM {
        cursor += SURFACE_PROPERTIES_BYTES;
    }

    let flags = read_u32(payload, 0)?;
    let triangles = read_u32(payload, GEOMETRY_TRIANGLES)?;
    let vertices = read_u32(payload, GEOMETRY_VERTICES)? as usize;
    let morph_targets = read_u32(payload, GEOMETRY_MORPH_TARGETS)?;

    let prelit = flags & FLAG_PRELIT != 0;
    let texcoord_sets = if flags & FLAG_TEXTURED2 != 0 {
        2
    } else {
        u8::from(flags & FLAG_TEXTURED != 0)
    };

    if prelit {
        cursor += COLOUR_BYTES * vertices;
    }

    cursor += TEXCOORD_BYTES * usize::from(texcoord_sets) * vertices;
    cursor += TRIANGLE_BYTES * triangles as usize;

    let mut targets_with_vertices = 0;
    let mut targets_with_normals = 0;

    for _ in 0..morph_targets {
        cursor += 16; // bounding sphere

        let has_vertices = read_u32(payload, cursor)?;
        let has_normals = read_u32(payload, cursor + 4)?;
        cursor += 8;

        if has_vertices != 0 {
            targets_with_vertices += 1;
            cursor += VECTOR_BYTES * vertices;
        }

        if has_normals != 0 {
            targets_with_normals += 1;
            cursor += VECTOR_BYTES * vertices;
        }
    }

    Ok(GeometryLayout {
        payload: payload.len(),
        consumed: cursor,
        prelit,
        texcoord_sets,
        morph_targets,
        targets_with_vertices,
        targets_with_normals,
    })
}

/// One vertex after decoding.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vertex {
    /// Position in model space.
    pub position: [f32; 3],
    /// Normal, zero when the geometry carries none.
    pub normal: [f32; 3],
    /// Texture coordinates per set, zero when the geometry carries none.
    pub texcoords: [[f32; 2]; 2],
    /// Prelit colour as stored, white when the geometry carries none.
    pub colour: [u8; 4],
}

impl Default for Vertex {
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            normal: [0.0; 3],
            texcoords: [[0.0; 2]; 2],
            colour: [255, 255, 255, 255],
        }
    }
}

/// One triangle and the material it belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Triangle {
    /// Vertex indices.
    pub indices: [u16; 3],
    /// Material index into the geometry's material list.
    pub material: u16,
}

/// A geometry's decoded geometry data.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedGeometry {
    /// Geometry flags, as stored.
    pub flags: u32,
    /// Vertices, in file order.
    pub vertices: Vec<Vertex>,
    /// Triangles, in file order.
    pub triangles: Vec<Triangle>,
    /// Whether the vertices carry prelit colours.
    pub has_colours: bool,
    /// Whether the vertices carry normals.
    pub has_normals: bool,
    /// Number of texcoord sets that were present.
    pub texcoord_sets: u8,
}

impl DecodedGeometry {
    /// Indices that point past the vertex list.
    pub fn out_of_range_indices(&self) -> usize {
        let limit = self.vertices.len() as u16;

        self.triangles
            .iter()
            .flat_map(|triangle| triangle.indices)
            .filter(|index| *index >= limit)
            .count()
    }

    /// Vertices whose position is not finite.
    pub fn non_finite_positions(&self) -> usize {
        self.vertices
            .iter()
            .filter(|vertex| vertex.position.iter().any(|value| !value.is_finite()))
            .count()
    }
}

/// Decodes the vertex and index data of one geometry struct.
///
/// # Errors
/// Returns [`DffError::PayloadTooShort`] when the struct ends before a field the
/// layout needs, and [`DffError::IndexOutOfRange`] when a triangle names a vertex
/// that does not exist.
pub fn decode_geometry(payload: &[u8], stamp: u32) -> Result<DecodedGeometry, DffError> {
    let flags = read_u32(payload, 0)?;
    let triangle_count = read_u32(payload, GEOMETRY_TRIANGLES)? as usize;
    let vertex_count = read_u32(payload, GEOMETRY_VERTICES)? as usize;
    let morph_targets = read_u32(payload, GEOMETRY_MORPH_TARGETS)?;

    let has_colours = flags & FLAG_PRELIT != 0;
    let texcoord_sets = if flags & FLAG_TEXTURED2 != 0 {
        2
    } else {
        u8::from(flags & FLAG_TEXTURED != 0)
    };

    let mut cursor = 16;

    if stamp < SURFACE_PROPERTIES_ABSENT_FROM {
        cursor += SURFACE_PROPERTIES_BYTES;
    }

    let mut vertices = vec![Vertex::default(); vertex_count];

    if has_colours {
        read_colours(payload, cursor, &mut vertices)?;
        cursor += COLOUR_BYTES * vertex_count;
    }

    if texcoord_sets != 0 {
        read_texcoords(payload, cursor, texcoord_sets, &mut vertices)?;
        cursor += TEXCOORD_BYTES * usize::from(texcoord_sets) * vertex_count;
    }

    let triangles = read_triangles(payload, cursor, triangle_count)?;
    cursor += TRIANGLE_BYTES * triangle_count;

    let has_normals = read_morph_targets(payload, cursor, morph_targets, &mut vertices)?;

    let decoded = DecodedGeometry {
        flags,
        vertices,
        triangles,
        has_colours,
        has_normals,
        texcoord_sets,
    };

    if decoded.out_of_range_indices() != 0 {
        return Err(DffError::IndexOutOfRange);
    }

    Ok(decoded)
}

fn read_colours(payload: &[u8], at: usize, vertices: &mut [Vertex]) -> Result<(), DffError> {
    for (index, vertex) in vertices.iter_mut().enumerate() {
        let start = at + COLOUR_BYTES * index;
        let bytes = payload
            .get(start..start + COLOUR_BYTES)
            .ok_or(DffError::PayloadTooShort { offset: start })?;

        vertex.colour = [bytes[0], bytes[1], bytes[2], bytes[3]];
    }

    Ok(())
}

fn read_texcoords(
    payload: &[u8],
    at: usize,
    sets: u8,
    vertices: &mut [Vertex],
) -> Result<(), DffError> {
    let count = vertices.len();

    for (index, vertex) in vertices.iter_mut().enumerate() {
        for set in 0..usize::from(sets) {
            let start = at + TEXCOORD_BYTES * (set * count + index);

            vertex.texcoords[set] = [read_f32(payload, start)?, read_f32(payload, start + 4)?];
        }
    }

    Ok(())
}

fn read_triangles(payload: &[u8], at: usize, count: usize) -> Result<Vec<Triangle>, DffError> {
    let mut triangles = Vec::with_capacity(count);

    for index in 0..count {
        let start = at + TRIANGLE_BYTES * index;
        let first = read_u32(payload, start)?;
        let second = read_u32(payload, start + 4)?;

        triangles.push(Triangle {
            indices: [(first >> 16) as u16, first as u16, (second >> 16) as u16],
            material: second as u16,
        });
    }

    Ok(triangles)
}

/// Reads the morph targets, returning whether any carried normals.
fn read_morph_targets(
    payload: &[u8],
    mut cursor: usize,
    count: u32,
    vertices: &mut [Vertex],
) -> Result<bool, DffError> {
    let mut has_normals = false;

    for _ in 0..count {
        cursor += 16; // bounding sphere

        let has_vertices = read_u32(payload, cursor)? != 0;
        let target_has_normals = read_u32(payload, cursor + 4)? != 0;
        cursor += 8;

        if has_vertices {
            for (index, vertex) in vertices.iter_mut().enumerate() {
                let start = cursor + VECTOR_BYTES * index;
                vertex.position = [
                    read_f32(payload, start)?,
                    read_f32(payload, start + 4)?,
                    read_f32(payload, start + 8)?,
                ];
            }

            cursor += VECTOR_BYTES * vertices.len();
        }

        if target_has_normals {
            has_normals = true;

            for (index, vertex) in vertices.iter_mut().enumerate() {
                let start = cursor + VECTOR_BYTES * index;
                vertex.normal = [
                    read_f32(payload, start)?,
                    read_f32(payload, start + 4)?,
                    read_f32(payload, start + 8)?,
                ];
            }

            cursor += VECTOR_BYTES * vertices.len();
        }
    }

    Ok(has_normals)
}

fn read_f32(payload: &[u8], offset: usize) -> Result<f32, DffError> {
    let field = payload
        .get(offset..offset + 4)
        .ok_or(DffError::PayloadTooShort { offset })?;

    Ok(f32::from_le_bytes([field[0], field[1], field[2], field[3]]))
}

fn read_u32(payload: &[u8], offset: usize) -> Result<u32, DffError> {
    let field = payload
        .get(offset..offset + 4)
        .ok_or(DffError::PayloadTooShort { offset })?;

    Ok(u32::from_le_bytes([field[0], field[1], field[2], field[3]]))
}

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
    /// Geometries whose struct layout accounted for every byte.
    pub layouts_exact: u32,
    /// Geometries whose struct layout could not be walked.
    pub layout_failures: u32,
    /// Geometries carrying prelit vertex colours.
    pub prelit_geometries: u32,
    /// Geometries carrying normals.
    pub normal_geometries: u32,
    /// Texcoord sets across all geometries.
    pub texcoord_sets: u64,
    /// Triangles produced by decoding.
    pub decoded_triangles: u64,
    /// Vertices produced by decoding.
    pub decoded_vertices: u64,
    /// Indices that pointed past their vertex list.
    pub decoded_index_errors: u64,
    /// Vertices whose position was not finite.
    pub decoded_non_finite: u64,
    /// Geometries whose data could not be decoded.
    pub decode_failures: u32,
    /// Material slots across all geometries.
    pub material_slots: u64,
    /// Slots that name a texture.
    pub named_slots: u64,
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

            if let Ok(materials) = read_materials(&geometry) {
                clump.material_slots += materials.len() as u64;
                clump.named_slots += materials
                    .iter()
                    .filter(|material| !material.texture.is_empty())
                    .count() as u64;
            }

            match decode_geometry(geometry_struct.payload(), geometry_struct.stamp().0) {
                Ok(decoded) => {
                    clump.decoded_triangles += decoded.triangles.len() as u64;
                    clump.decoded_vertices += decoded.vertices.len() as u64;
                    clump.decoded_index_errors += decoded.out_of_range_indices() as u64;
                    clump.decoded_non_finite += decoded.non_finite_positions() as u64;
                }
                Err(_) => clump.decode_failures += 1,
            }

            match read_layout(geometry_struct.payload(), geometry_struct.stamp().0) {
                Ok(layout) => {
                    if layout.is_exact() {
                        clump.layouts_exact += 1;
                    }

                    if layout.prelit {
                        clump.prelit_geometries += 1;
                    }

                    if layout.targets_with_normals != 0 {
                        clump.normal_geometries += 1;
                    }

                    clump.texcoord_sets += u64::from(layout.texcoord_sets);
                }
                Err(_) => clump.layout_failures += 1,
            }

            if let Some(material_list) = geometry.child(SectionKind::MATERIAL_LIST)? {
                clump.materials += material_list.children_of(SectionKind::MATERIAL)?.len() as u32;
            }
        }
    }

    Ok(clump)
}

/// One material slot of a geometry: the texture it uses and its alpha mask.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MaterialRef {
    /// Diffuse texture name, empty when the slot carries none.
    pub texture: String,
    /// Alpha mask texture name, empty when there is none.
    pub alpha_mask: String,
    /// Material colour, multiplied into the surface the way the original did it.
    ///
    /// A slot with no texture draws in exactly this colour, which is how the beach and
    /// a lot of road paint are coloured - the pale sand has no texture to inherit from.
    pub colour: [f32; 4],
}

/// Bytes per frame in a frame list: a 3x3 rotation, a position, a parent index and
/// a matrix flags word.
pub const FRAME_BYTES: usize = 56;

/// A geometry with the materials its triangles index into.
#[derive(Debug, Clone, PartialEq)]
pub struct ClumpGeometry {
    /// Vertex and index data.
    pub decoded: DecodedGeometry,
    /// Material slots, in the order the triangles index them.
    pub materials: Vec<MaterialRef>,
}

/// One frame of a clump's hierarchy: a rotation and a position, relative to its parent.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    /// Rotation matrix, row-major.
    pub rotation: [[f32; 3]; 3],
    /// Position relative to the parent frame.
    pub position: [f32; 3],
    /// Parent frame, or -1 at the root.
    pub parent: i32,
}

/// One atomic: a geometry attached to a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtomicRef {
    /// Frame the geometry hangs from.
    pub frame: u32,
    /// Geometry it draws.
    pub geometry: u32,
}

/// A clump's geometry together with the hierarchy that positions it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DecodedClump {
    /// Geometries, in list order.
    pub geometries: Vec<ClumpGeometry>,
    /// Frames, root first.
    pub frames: Vec<Frame>,
    /// Atomics, in list order.
    pub atomics: Vec<AtomicRef>,
}

impl DecodedClump {
    /// Rotation and position of every geometry, in model space.
    ///
    /// A geometry's own vertices are in its frame's space, so this is the transform
    /// that has to be applied before the instance's. Ignoring it is what leaves street
    /// lights standing apart from their posts and signs rotated against their frames.
    pub fn geometry_transforms(&self) -> Vec<([[f32; 3]; 3], [f32; 3])> {
        let mut transforms = vec![(identity(), [0.0; 3]); self.geometries.len()];

        for atomic in &self.atomics {
            let Some(slot) = transforms.get_mut(atomic.geometry as usize) else {
                continue;
            };

            *slot = self.frame_transform(atomic.frame, 0);
        }

        transforms
    }

    fn frame_transform(&self, frame: u32, depth: u8) -> ([[f32; 3]; 3], [f32; 3]) {
        let Some(current) = self.frames.get(frame as usize) else {
            return (identity(), [0.0; 3]);
        };

        if current.parent < 0 || depth > 32 {
            return (current.rotation, current.position);
        }

        let (parent_rotation, parent_position) =
            self.frame_transform(current.parent as u32, depth + 1);

        (
            multiply(parent_rotation, current.rotation),
            add(parent_position, rotate(parent_rotation, current.position)),
        )
    }
}

/// Identity rotation.
pub const fn identity() -> [[f32; 3]; 3] {
    [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]
}

/// Matrix product `a * b`, both row-major.
pub(crate) fn multiply(a: [[f32; 3]; 3], b: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut out = [[0.0; 3]; 3];

    for row in 0..3 {
        for column in 0..3 {
            out[row][column] = (0..3).map(|k| a[row][k] * b[k][column]).sum();
        }
    }

    out
}

#[expect(
    clippy::suboptimal_flops,
    reason = "textbook matrix arithmetic reads better than mul_add chains"
)]
/// Applies a rotation matrix to a point.
pub fn rotate(m: [[f32; 3]; 3], p: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * p[0] + m[0][1] * p[1] + m[0][2] * p[2],
        m[1][0] * p[0] + m[1][1] * p[1] + m[1][2] * p[2],
        m[2][0] * p[0] + m[2][1] * p[1] + m[2][2] * p[2],
    ]
}

/// Adds two points.
pub fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// Reads a geometry's material slots.
///
/// # Errors
/// Propagates [`StreamError`] for a malformed tree.
pub fn read_materials(geometry: &Section<'_>) -> Result<Vec<MaterialRef>, DffError> {
    // The list hangs directly off the geometry; the extension hop is a fallback
    // because some exporters nest it there instead.
    let list = if let Some(list) = geometry.child(SectionKind::MATERIAL_LIST).unwrap_or(None) {
        list
    } else {
        let Ok(Some(extension)) = geometry.child(SectionKind::EXTENSION) else {
            return Ok(Vec::new());
        };
        let Ok(Some(nested)) = extension.child(SectionKind::MATERIAL_LIST) else {
            return Ok(Vec::new());
        };

        nested
    };

    let mut materials = Vec::new();

    // Plugin chunks inside a material do not always parse as sections, so the walk
    // is best-effort: a material that cannot be read keeps its slot, with no name.
    for material in list.children_of(SectionKind::MATERIAL).unwrap_or_default() {
        let mut names = Vec::new();
        let colour = material_colour(&material);

        for texture in material
            .children_of(SectionKind::TEXTURE)
            .unwrap_or_default()
        {
            for string in texture.children_of(SectionKind::STRING).unwrap_or_default() {
                if let Some(name) = printable(&string) {
                    names.push(name);
                }
            }
        }

        materials.push(MaterialRef {
            texture: names.first().cloned().unwrap_or_default(),
            alpha_mask: names.get(1).cloned().unwrap_or_default(),
            colour,
        });
    }

    Ok(materials)
}

/// The colour of a material.
///
/// A material chunk is a container whose `STRUCT` child holds
/// `{ i32 flags; RGBA colour; i32 unused; i32 is_textured }`, so the colour is four
/// bytes at offset 4 of that struct. Reading the container's own payload reads the
/// chunk header instead, which is how every surface once came out the same shade.
fn material_colour(material: &Section<'_>) -> [f32; 4] {
    let Some(data) = material.child(SectionKind::STRUCT).unwrap_or(None) else {
        return [1.0; 4];
    };
    let Some(bytes) = data.payload().get(4..8) else {
        return [1.0; 4];
    };

    [
        f32::from(bytes[0]) / 255.0,
        f32::from(bytes[1]) / 255.0,
        f32::from(bytes[2]) / 255.0,
        f32::from(bytes[3]) / 255.0,
    ]
}

/// Reads a string chunk as NUL-terminated ASCII, ignoring binary payloads.
fn printable(section: &Section<'_>) -> Option<String> {
    let payload = section.payload();
    let end = payload
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(payload.len());
    let text = payload.get(..end)?;

    if text.is_empty() || !text.iter().all(u8::is_ascii_graphic) {
        return None;
    }

    Some(String::from_utf8_lossy(text).into_owned())
}

/// Reads the frame hierarchy and the atomics that attach geometry to it.
///
/// # Errors
/// Propagates [`StreamError`] for a malformed tree.
pub fn read_hierarchy(root: &Section<'_>) -> Result<(Vec<Frame>, Vec<AtomicRef>), DffError> {
    let mut frames = Vec::new();

    if let Some(frame_list) = root.child(SectionKind::FRAME_LIST)?
        && let Some(structure) = frame_list.child(SectionKind::STRUCT)?
    {
        let payload = structure.payload();
        let count = read_u32(payload, 0)? as usize;

        for index in 0..count {
            let at = 4 + index * FRAME_BYTES;

            if at + FRAME_BYTES > payload.len() {
                break;
            }

            frames.push(Frame {
                rotation: [
                    [
                        read_f32(payload, at)?,
                        read_f32(payload, at + 4)?,
                        read_f32(payload, at + 8)?,
                    ],
                    [
                        read_f32(payload, at + 12)?,
                        read_f32(payload, at + 16)?,
                        read_f32(payload, at + 20)?,
                    ],
                    [
                        read_f32(payload, at + 24)?,
                        read_f32(payload, at + 28)?,
                        read_f32(payload, at + 32)?,
                    ],
                ],
                position: [
                    read_f32(payload, at + 36)?,
                    read_f32(payload, at + 40)?,
                    read_f32(payload, at + 44)?,
                ],
                parent: read_u32(payload, at + 48)?.cast_signed(),
            });
        }
    }

    let mut atomics = Vec::new();

    for atomic in root.children_of(SectionKind::ATOMIC)? {
        if let Some(structure) = atomic.child(SectionKind::STRUCT)? {
            let payload = structure.payload();

            if payload.len() >= 8 {
                atomics.push(AtomicRef {
                    frame: read_u32(payload, 0)?,
                    geometry: read_u32(payload, 4)?,
                });
            }
        }
    }

    Ok((frames, atomics))
}

/// Every geometry in a clump with its materials and hierarchy, decoded.
///
/// # Errors
/// As [`decode_clump_with_materials`], plus any frame list that does not parse.
pub fn decode_clump_geometry(bytes: &[u8]) -> Result<DecodedClump, DffError> {
    let root = Section::root(bytes)?;

    if root.kind() != SectionKind::CLUMP {
        return Err(DffError::UnexpectedRoot { kind: root.kind() });
    }

    let geometries = decode_clump_with_materials(bytes)?;
    let (frames, atomics) = read_hierarchy(&root)?;

    Ok(DecodedClump {
        geometries,
        frames,
        atomics,
    })
}

/// Every geometry in a clump with its materials, decoded.
///
/// # Errors
/// As [`decode_clump`], plus any material tree that does not parse.
pub fn decode_clump_with_materials(bytes: &[u8]) -> Result<Vec<ClumpGeometry>, DffError> {
    let root = Section::root(bytes)?;

    if root.kind() != SectionKind::CLUMP {
        return Err(DffError::UnexpectedRoot { kind: root.kind() });
    }

    let mut geometries = Vec::new();

    for geometry_list in root.children_of(SectionKind::GEOMETRY_LIST)? {
        for geometry in geometry_list.children_of(SectionKind::GEOMETRY)? {
            let geometry_struct =
                geometry
                    .child(SectionKind::STRUCT)?
                    .ok_or(DffError::MissingStruct {
                        parent: SectionKind::GEOMETRY,
                    })?;

            geometries.push(ClumpGeometry {
                decoded: decode_geometry(geometry_struct.payload(), geometry_struct.stamp().0)?,
                materials: read_materials(&geometry)?,
            });
        }
    }

    Ok(geometries)
}

/// Every geometry in a clump, decoded.
///
/// # Errors
/// Returns [`DffError::UnexpectedRoot`] when the file is not a clump,
/// [`DffError::MissingStruct`] when a geometry list or geometry has no struct,
/// and propagates [`StreamError`] for malformed trees.
pub fn decode_clump(bytes: &[u8]) -> Result<Vec<DecodedGeometry>, DffError> {
    let root = Section::root(bytes)?;

    if root.kind() != SectionKind::CLUMP {
        return Err(DffError::UnexpectedRoot { kind: root.kind() });
    }

    let mut geometries = Vec::new();

    for geometry_list in root.children_of(SectionKind::GEOMETRY_LIST)? {
        for geometry in geometry_list.children_of(SectionKind::GEOMETRY)? {
            let geometry_struct =
                geometry
                    .child(SectionKind::STRUCT)?
                    .ok_or(DffError::MissingStruct {
                        parent: SectionKind::GEOMETRY,
                    })?;

            geometries.push(decode_geometry(
                geometry_struct.payload(),
                geometry_struct.stamp().0,
            )?);
        }
    }

    Ok(geometries)
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
    /// A triangle names a vertex that does not exist.
    #[error("triangle indices run past the vertex list")]
    IndexOutOfRange,
    /// A struct ended before a field the layout needs.
    #[error("struct payload has no field at offset {offset}")]
    PayloadTooShort {
        /// Requested offset.
        offset: usize,
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

    /// Builds a struct payload: flags, counts, then the fields the flags imply.
    fn layout_payload(
        flags: u32,
        triangles: u32,
        vertices: u32,
        with_vertices: bool,
        surface_properties: bool,
    ) -> Vec<u8> {
        let mut payload = flags.to_le_bytes().to_vec();
        payload.extend(triangles.to_le_bytes());
        payload.extend(vertices.to_le_bytes());
        payload.extend(1u32.to_le_bytes());

        if surface_properties {
            payload.resize(payload.len() + SURFACE_PROPERTIES_BYTES, 0);
        }

        if flags & FLAG_PRELIT != 0 {
            payload.resize(payload.len() + COLOUR_BYTES * vertices as usize, 0);
        }

        let sets = if flags & FLAG_TEXTURED2 != 0 { 2 } else { 1 };

        payload.resize(payload.len() + TEXCOORD_BYTES * sets * vertices as usize, 0);
        payload.resize(payload.len() + TRIANGLE_BYTES * triangles as usize, 0);
        payload.resize(payload.len() + 16, 0); // bounding sphere

        payload.extend(u32::from(with_vertices).to_le_bytes());
        payload.extend(0u32.to_le_bytes()); // no normals

        if with_vertices {
            payload.resize(payload.len() + VECTOR_BYTES * vertices as usize, 0);
        }

        payload
    }

    #[test]
    fn accounts_for_a_struct_without_surface_properties() {
        let payload = layout_payload(0x10037, 1, 3, true, false);

        let layout = read_layout(&payload, 0x1003_FFFF).unwrap();

        assert!(
            layout.is_exact(),
            "consumed {} of {}",
            layout.consumed,
            payload.len()
        );
        assert_eq!(layout.texcoord_sets, 1);
        assert_eq!(layout.targets_with_vertices, 1);
        assert!(!layout.prelit);
    }

    #[test]
    fn accounts_for_a_struct_with_surface_properties() {
        let payload = layout_payload(0x10037, 1, 3, true, true);

        let layout = read_layout(&payload, 0x0C02_FFFF).unwrap();

        assert!(layout.is_exact());
    }

    #[test]
    fn a_wrong_record_expectation_is_not_exact() {
        let payload = layout_payload(0x10037, 1, 3, true, false);

        let layout = read_layout(&payload, 0x0C02_FFFF).unwrap();

        assert!(
            !layout.is_exact(),
            "reading a struct as if it carried the record must not add up"
        );
    }

    #[test]
    fn accounts_for_prelit_colours_and_two_texcoord_sets() {
        let payload = layout_payload(FLAG_PRELIT | FLAG_TEXTURED2 | 0x02, 2, 4, true, false);

        let layout = read_layout(&payload, 0x1003_FFFF).unwrap();

        assert!(layout.is_exact());
        assert!(layout.prelit);
        assert_eq!(layout.texcoord_sets, 2);
    }

    /// Builds a one-triangle geometry carrying colours, texcoords and positions.
    fn decodable_payload() -> Vec<u8> {
        let mut payload = (FLAG_PRELIT | FLAG_TEXTURED | 0x02u32)
            .to_le_bytes()
            .to_vec();
        payload.extend(1u32.to_le_bytes()); // one triangle
        payload.extend(3u32.to_le_bytes()); // three vertices
        payload.extend(1u32.to_le_bytes()); // one morph target

        // Prelit colours: three distinct vertices.
        for shade in [10u8, 20, 30] {
            payload.extend([shade, shade, shade, 255]);
        }

        // Texcoords: vertex 0 at (0, 0), vertex 1 at (1, 0), vertex 2 at (0, 1).
        for uv in [[0.0f32, 0.0], [1.0, 0.0], [0.0, 1.0]] {
            payload.extend(uv[0].to_le_bytes());
            payload.extend(uv[1].to_le_bytes());
        }

        // One triangle: vertices 0, 1, 2 with material 7.
        payload.extend(1u32.to_le_bytes()); // v0 = 0, v1 = 1
        payload.extend(0x0002_0007u32.to_le_bytes()); // v2 = 2, material = 7

        payload.extend([0u8; 16]); // bounding sphere
        payload.extend(1i32.to_le_bytes()); // has vertices
        payload.extend(0i32.to_le_bytes()); // no normals

        for position in [[1.0f32, 2.0, 3.0], [4.0, 5.0, 6.0], [7.0, 8.0, 9.0]] {
            for axis in position {
                payload.extend(axis.to_le_bytes());
            }
        }

        payload
    }

    #[test]
    fn decodes_positions_colours_and_texcoords() {
        let decoded = decode_geometry(&decodable_payload(), 0x1003_FFFF).unwrap();

        assert_eq!(decoded.vertices.len(), 3);
        assert_eq!(decoded.vertices[0].position, [1.0, 2.0, 3.0]);
        assert_eq!(decoded.vertices[2].position, [7.0, 8.0, 9.0]);
        assert_eq!(decoded.vertices[0].colour, [10, 10, 10, 255]);
        assert_eq!(decoded.vertices[1].texcoords[0], [1.0, 0.0]);
        assert!(decoded.has_colours);
        assert_eq!(decoded.texcoord_sets, 1);
        assert!(!decoded.has_normals);
    }

    #[test]
    fn decodes_a_triangle_and_its_material() {
        let decoded = decode_geometry(&decodable_payload(), 0x1003_FFFF).unwrap();

        assert_eq!(decoded.triangles.len(), 1);
        assert_eq!(decoded.triangles[0].indices, [0, 1, 2]);
        assert_eq!(decoded.triangles[0].material, 7);
    }

    #[test]
    fn rejects_a_triangle_that_names_a_missing_vertex() {
        let mut payload = decodable_payload();
        // Overwrite the second triangle word so v2 points at vertex 9.
        payload[56..60].copy_from_slice(&0x0009_0007u32.to_le_bytes());

        let result = decode_geometry(&payload, 0x1003_FFFF);

        assert_eq!(result.unwrap_err(), DffError::IndexOutOfRange);
    }

    #[test]
    fn reports_a_struct_that_ends_before_a_morph_target() {
        let mut payload = layout_payload(0x10037, 0, 0, false, false);
        payload.truncate(24); // the counts say one morph target, the bytes are gone

        let result = read_layout(&payload, 0x1003_FFFF);

        assert!(matches!(result, Err(DffError::PayloadTooShort { .. })));
    }
}
