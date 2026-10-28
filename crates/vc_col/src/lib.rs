//! Reader for Vice City collision files.
//!
//! A collision file is a flat sequence of model records, each introduced by the
//! `COLL` magic and a declared payload length. The length covers the name, the
//! model id and the body, so a record spans `8 + size` bytes and records are
//! stepped by that, never by parsing the header fields: the bytes after the name
//! are not stable in shipped data, because long model names overflow into them.
//!
//! Each body holds a bounding sphere and box, then sphere, box, vertex and face
//! lists with per-face surface properties.
//!
//! # Examples
//!
//! ```
//! use vc_col::Collision;
//!
//! let mut bytes = Vec::new();
//! bytes.extend(b"COLL");
//! bytes.extend(84u32.to_le_bytes()); // name, id and body
//! let mut header = vec![0u8; 24]; // name[22] and model id
//! header[..7].copy_from_slice(b"box_a01");
//! bytes.extend(header);
//! bytes.extend([0u8; 60]); // bounds, then five zero counts
//!
//! let collision = Collision::parse(&bytes)?;
//! assert_eq!(collision.records[0].name, "box_a01");
//! # Ok::<(), vc_col::CollError>(())
//! ```

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

use thiserror::Error;

/// Magic bytes that introduce every record.
pub const MAGIC: &[u8; 4] = b"COLL";

/// Bytes of a record before the body: magic, declared size, name and model id.
pub const HEADER_SIZE: usize = 32;

/// Bytes of the bounding sphere and box that follow the header.
pub const BOUNDS_SIZE: usize = 40;

/// Bytes per collision sphere.
pub const SPHERE_SIZE: usize = 20;

/// Bytes per collision box.
pub const BOX_SIZE: usize = 28;

/// Bytes per collision vertex.
pub const VERTEX_SIZE: usize = 12;

/// Bytes per collision face.
pub const FACE_SIZE: usize = 16;

/// Bounding sphere and box of a collision model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    /// Centre of the bounding sphere.
    pub center: [f32; 3],
    /// Radius of the bounding sphere.
    pub radius: f32,
    /// Minimum corner of the bounding box.
    pub min: [f32; 3],
    /// Maximum corner of the bounding box.
    pub max: [f32; 3],
}

/// A collision sphere.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sphere {
    /// Centre in model space.
    pub centre: [f32; 3],
    /// Radius.
    pub radius: f32,
    /// Surface the sphere reports.
    pub surface: u32,
}

/// A collision box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Box3 {
    /// Minimum corner.
    pub min: [f32; 3],
    /// Maximum corner.
    pub max: [f32; 3],
    /// Surface the box reports.
    pub surface: u32,
}

/// One collision face.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Face {
    /// Vertex indices.
    pub indices: [u16; 3],
    /// Surface the face reports.
    pub surface: u16,
}

/// One collision model.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    /// Model name, as stored in the file.
    pub name: String,
    /// Declared payload length, covering the name, model id and body.
    pub size: u32,
    /// Bounding volume, absent when the record is too short to hold one.
    pub bounds: Option<Bounds>,
    /// Number of collision spheres.
    pub spheres: u32,
    /// Number of collision boxes.
    pub boxes: u32,
    /// Number of collision vertices.
    pub vertices: u32,
    /// Number of collision faces.
    pub faces: u32,
    /// Bytes between the end of the parsed body and the declared end of the record.
    pub unexplained_tail: u32,
    /// Spheres, when the file carries them.
    pub sphere_list: Vec<Sphere>,
    /// Boxes, when the file carries them.
    pub box_list: Vec<Box3>,
    /// Mesh vertices.
    pub vertex_list: Vec<[f32; 3]>,
    /// Mesh faces.
    pub face_list: Vec<Face>,
}

/// A parsed collision file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Collision {
    /// Collision models, in file order.
    pub records: Vec<Record>,
}

impl Collision {
    /// Parses every record in `bytes`.
    ///
    /// # Errors
    /// Returns [`CollError::InvalidMagic`] when a record does not start with
    /// `COLL`, [`CollError::Truncated`] when a field runs past the input, and
    /// [`CollError::MalformedRecord`] when a record's lists extend past its
    /// declared length.
    pub fn parse(bytes: &[u8]) -> Result<Self, CollError> {
        let mut records = Vec::new();
        let mut offset = 0;

        while offset < bytes.len() {
            if bytes.get(offset..offset + 4) != Some(MAGIC) {
                return Err(CollError::InvalidMagic { offset });
            }

            if offset + HEADER_SIZE > bytes.len() {
                return Err(CollError::Truncated {
                    offset,
                    needed: offset + HEADER_SIZE,
                    len: bytes.len(),
                });
            }

            let size = read_u32(bytes, offset + 4)?;
            let declared_end = offset + 8 + size as usize;

            if declared_end > bytes.len() {
                return Err(CollError::Truncated {
                    offset,
                    needed: declared_end,
                    len: bytes.len(),
                });
            }

            records.push(parse_record(bytes, offset, declared_end)?);
            offset = declared_end;
        }

        Ok(Self { records })
    }

    /// Parses records until one does not parse, reporting the bytes left behind.
    ///
    /// Shipped files carry trailing data that is not a record — `airport.col` has
    /// 3 KB of it — so a caller that wants every model in the file stops there
    /// rather than losing the whole file. The count is returned so it can be
    /// reported instead of silently swallowed.
    pub fn parse_lenient(bytes: &[u8]) -> (Self, usize) {
        let mut records = Vec::new();
        let mut offset = 0;

        while offset < bytes.len() {
            let ok = bytes.get(offset..offset + 4) == Some(MAGIC)
                && offset + HEADER_SIZE <= bytes.len()
                && read_u32(bytes, offset + 4)
                    .is_ok_and(|size| offset + 8 + size as usize <= bytes.len());

            if !ok {
                break;
            }

            let Ok(size) = read_u32(bytes, offset + 4) else {
                break;
            };
            let declared_end = offset + 8 + size as usize;

            match parse_record(bytes, offset, declared_end) {
                Ok(record) => records.push(record),
                Err(_) => break,
            }

            offset = declared_end;
        }

        (Self { records }, bytes.len() - offset)
    }

    /// Summed contents of every record.
    pub fn totals(&self) -> Totals {
        self.records
            .iter()
            .fold(Totals::default(), |mut totals, record| {
                totals.spheres += u64::from(record.spheres);
                totals.boxes += u64::from(record.boxes);
                totals.vertices += u64::from(record.vertices);
                totals.faces += u64::from(record.faces);
                totals
            })
    }

    /// Number of records whose body did not consume its declared length.
    pub fn records_with_tail(&self) -> usize {
        self.records
            .iter()
            .filter(|record| record.unexplained_tail != 0)
            .count()
    }

    /// Number of records too short to carry a bounding volume.
    pub fn records_without_bounds(&self) -> usize {
        self.records
            .iter()
            .filter(|record| record.bounds.is_none())
            .count()
    }
}

/// Summed contents of a collision file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Totals {
    /// Collision spheres.
    pub spheres: u64,
    /// Collision boxes.
    pub boxes: u64,
    /// Collision vertices.
    pub vertices: u64,
    /// Collision faces.
    pub faces: u64,
}

fn parse_record(bytes: &[u8], offset: usize, declared_end: usize) -> Result<Record, CollError> {
    let size = read_u32(bytes, offset + 4)?;
    let name = read_name(bytes, offset + 8, 22);
    let header_end = offset + HEADER_SIZE;

    let bounds = read_bounds(bytes, header_end);
    let mut cursor = header_end + BOUNDS_SIZE;

    let spheres = read_count(bytes, cursor, declared_end, offset)?;
    let sphere_end = list_end(cursor, spheres, SPHERE_SIZE, declared_end, offset)?;
    let sphere_start = cursor + 4;
    let mut sphere_list = Vec::with_capacity(spheres as usize);

    for index in 0..spheres as usize {
        let at = sphere_start + SPHERE_SIZE * index;
        sphere_list.push(Sphere {
            centre: [
                read_f32(bytes, at)?,
                read_f32(bytes, at + 4)?,
                read_f32(bytes, at + 8)?,
            ],
            radius: read_f32(bytes, at + 12)?,
            surface: read_u32(bytes, at + 16)?,
        });
    }

    cursor = sphere_end;

    cursor += 4; // reserved field between the sphere and box lists

    let boxes = read_count(bytes, cursor, declared_end, offset)?;
    let box_end = list_end(cursor, boxes, BOX_SIZE, declared_end, offset)?;
    let box_start = cursor + 4;
    let mut box_list = Vec::with_capacity(boxes as usize);

    for index in 0..boxes as usize {
        let at = box_start + BOX_SIZE * index;
        box_list.push(Box3 {
            min: [
                read_f32(bytes, at)?,
                read_f32(bytes, at + 4)?,
                read_f32(bytes, at + 8)?,
            ],
            max: [
                read_f32(bytes, at + 12)?,
                read_f32(bytes, at + 16)?,
                read_f32(bytes, at + 20)?,
            ],
            surface: read_u32(bytes, at + 24)?,
        });
    }

    cursor = box_end;

    let vertices = read_count(bytes, cursor, declared_end, offset)?;
    let vertex_end = list_end(cursor, vertices, VERTEX_SIZE, declared_end, offset)?;
    let vertex_start = cursor + 4;
    let mut vertex_list = Vec::with_capacity(vertices as usize);

    for index in 0..vertices as usize {
        let at = vertex_start + VERTEX_SIZE * index;
        vertex_list.push([
            read_f32(bytes, at)?,
            read_f32(bytes, at + 4)?,
            read_f32(bytes, at + 8)?,
        ]);
    }

    cursor = vertex_end;

    let faces = read_count(bytes, cursor, declared_end, offset)?;
    let face_end = list_end(cursor, faces, FACE_SIZE, declared_end, offset)?;
    let face_start = cursor + 4;
    let mut face_list = Vec::with_capacity(faces as usize);

    for index in 0..faces as usize {
        let at = face_start + FACE_SIZE * index;
        let (first, second) = read_u16x2(bytes, at)?;
        let third = read_u16x2(bytes, at + 4)?.0;
        let surface = read_u16x2(bytes, at + 4)?.1;

        face_list.push(Face {
            indices: [first, second, third],
            surface,
        });
    }

    cursor = face_end;

    if cursor > declared_end {
        return Err(CollError::MalformedRecord {
            offset,
            consumed: cursor - offset,
            declared: declared_end - offset,
        });
    }

    Ok(Record {
        name,
        size,
        bounds,
        spheres,
        boxes,
        vertices,
        faces,
        unexplained_tail: (declared_end - cursor) as u32,
        sphere_list,
        box_list,
        vertex_list,
        face_list,
    })
}

/// Advances past a list, refusing one that would run past the record.
const fn list_end(
    cursor: usize,
    count: u32,
    entry: usize,
    declared_end: usize,
    offset: usize,
) -> Result<usize, CollError> {
    let end = cursor + 4 + entry * count as usize;

    if end > declared_end {
        return Err(CollError::MalformedRecord {
            offset,
            consumed: end - offset,
            declared: declared_end - offset,
        });
    }

    Ok(end)
}

fn read_f32(bytes: &[u8], offset: usize) -> Result<f32, CollError> {
    let field = bytes.get(offset..offset + 4).ok_or(CollError::Truncated {
        offset,
        needed: offset + 4,
        len: bytes.len(),
    })?;

    Ok(f32::from_le_bytes([field[0], field[1], field[2], field[3]]))
}

fn read_u16x2(bytes: &[u8], offset: usize) -> Result<(u16, u16), CollError> {
    let field = bytes.get(offset..offset + 4).ok_or(CollError::Truncated {
        offset,
        needed: offset + 4,
        len: bytes.len(),
    })?;

    Ok((
        u16::from_le_bytes([field[0], field[1]]),
        u16::from_le_bytes([field[2], field[3]]),
    ))
}

/// Reads a list count, refusing one that cannot fit inside the record.
fn read_count(
    bytes: &[u8],
    cursor: usize,
    declared_end: usize,
    offset: usize,
) -> Result<u32, CollError> {
    if cursor + 4 > declared_end {
        return Err(CollError::MalformedRecord {
            offset,
            consumed: cursor - offset,
            declared: declared_end - offset,
        });
    }

    read_u32(bytes, cursor)
}

fn read_bounds(bytes: &[u8], offset: usize) -> Option<Bounds> {
    let floats = bytes.get(offset..offset + BOUNDS_SIZE)?;
    let mut values = [0f32; 10];

    for (index, slot) in values.iter_mut().enumerate() {
        let at = index * 4;
        *slot = f32::from_le_bytes([floats[at], floats[at + 1], floats[at + 2], floats[at + 3]]);
    }

    let bounds = Bounds {
        center: [values[0], values[1], values[2]],
        radius: values[3],
        min: [values[4], values[5], values[6]],
        max: [values[7], values[8], values[9]],
    };

    let finite = bounds
        .center
        .iter()
        .chain(bounds.min.iter())
        .chain(bounds.max.iter())
        .all(|value| value.is_finite())
        && bounds.radius.is_finite();

    finite.then_some(bounds)
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, CollError> {
    let field = bytes.get(offset..offset + 4).ok_or(CollError::Truncated {
        offset,
        needed: offset + 4,
        len: bytes.len(),
    })?;

    Ok(u32::from_le_bytes([field[0], field[1], field[2], field[3]]))
}

fn read_name(bytes: &[u8], offset: usize, width: usize) -> String {
    let Some(field) = bytes.get(offset..offset + width) else {
        return String::new();
    };

    field
        .iter()
        .take_while(|byte| **byte != 0 && byte.is_ascii_graphic())
        .map(|byte| char::from(*byte))
        .collect()
}

/// Failures while reading a collision file.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CollError {
    /// A record does not start with the expected magic.
    #[error("record at offset {offset} does not start with COLL")]
    InvalidMagic {
        /// Byte offset of the record.
        offset: usize,
    },
    /// A field runs past the end of the file.
    #[error("field at offset {offset} needs {needed} bytes, input is {len}")]
    Truncated {
        /// Byte offset of the field.
        offset: usize,
        /// Byte offset the field needs to reach.
        needed: usize,
        /// Length of the input.
        len: usize,
    },
    /// A record's lists extend past its declared length.
    #[error("record at offset {offset} consumed {consumed} bytes but declared {declared}")]
    MalformedRecord {
        /// Byte offset of the record.
        offset: usize,
        /// Bytes the parsed body needed.
        consumed: usize,
        /// Bytes the record declared.
        declared: usize,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(name: &str, payload: &[u8]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend(MAGIC);
        bytes.extend((24u32 + payload.len() as u32).to_le_bytes());
        bytes.extend(name.as_bytes());
        bytes.resize(32, 0);
        bytes.extend(payload);
        bytes
    }

    fn empty_body() -> Vec<u8> {
        let mut body = Vec::new();
        body.extend([0u8; BOUNDS_SIZE]);
        body.extend(0u32.to_le_bytes()); // spheres
        body.extend(0u32.to_le_bytes()); // reserved
        body.extend(0u32.to_le_bytes()); // boxes
        body.extend(0u32.to_le_bytes()); // vertices
        body.extend(0u32.to_le_bytes()); // faces
        body
    }

    #[test]
    fn reads_a_record_name() {
        let bytes = record("box_a01", &empty_body());

        let collision = Collision::parse(&bytes).unwrap();

        assert_eq!(collision.records[0].name, "box_a01");
    }

    #[test]
    fn steps_over_records_of_differing_length() {
        let mut bytes = record("first", &empty_body());
        bytes.extend(record("second", &empty_body()));

        let collision = Collision::parse(&bytes).unwrap();

        assert_eq!(collision.records.len(), 2);
    }

    #[test]
    fn rejects_a_file_without_the_magic() {
        let bytes = b"NOPE".to_vec();

        let result = Collision::parse(&bytes);

        assert_eq!(result.unwrap_err(), CollError::InvalidMagic { offset: 0 });
    }

    #[test]
    fn rejects_a_header_that_is_cut_short() {
        let bytes = MAGIC.to_vec();

        let result = Collision::parse(&bytes);

        assert!(matches!(
            result,
            Err(CollError::Truncated { offset: 0, .. })
        ));
    }

    #[test]
    fn accepts_an_empty_file() {
        let collision = Collision::parse(&[]).unwrap();

        assert_eq!(collision.records, Vec::new());
    }

    #[test]
    fn rejects_a_record_whose_lists_run_past_its_length() {
        let mut body = Vec::new();
        body.extend([0u8; BOUNDS_SIZE]);
        body.extend(64u32.to_le_bytes()); // 64 spheres, far past the record
        let bytes = record("overrun", &body);

        let result = Collision::parse(&bytes);

        assert!(matches!(result, Err(CollError::MalformedRecord { .. })));
    }

    #[test]
    fn reports_a_body_that_does_not_fill_the_record() {
        let mut body = empty_body();
        body.extend([0u8; 8]);
        let bytes = record("padded", &body);

        let collision = Collision::parse(&bytes).unwrap();

        assert_eq!(collision.records[0].unexplained_tail, 8);
        assert_eq!(collision.records_with_tail(), 1);
    }

    #[test]
    fn counts_a_sphere_list() {
        let mut body = Vec::new();
        body.extend([0u8; BOUNDS_SIZE]);
        body.extend(1u32.to_le_bytes()); // one sphere
        body.extend([0u8; SPHERE_SIZE]);
        body.extend(0u32.to_le_bytes()); // reserved
        body.extend(0u32.to_le_bytes()); // boxes
        body.extend(0u32.to_le_bytes()); // vertices
        body.extend(0u32.to_le_bytes()); // faces
        let bytes = record("sphere", &body);

        let collision = Collision::parse(&bytes).unwrap();

        assert_eq!(collision.records[0].spheres, 1);
        assert_eq!(collision.totals().spheres, 1);
    }
}
