//! Object definitions and placements.
//!
//! IDE files define what a model id means; IPL files place those ids in the world. Together they are
//! the map: 3,789 definitions against 9,114 placements in the shipped data.
//!
//! A VC placement line carries thirteen values — id, model name, interior, position, scale and a
//! quaternion — which is one more triple than the earlier games' format and the reason the rotation
//! below is a quaternion rather than an Euler triple.
//!
//! # Examples
//!
//! ```
//! use vc_world::{Instance, parse_instances};
//!
//! let text = "inst\n451, veg_palm, 0, 1.0, 2.0, 3.0, 1, 1, 1, 0, 0, 0, 1\nend\n";
//! let instances = parse_instances(text)?;
//!
//! assert_eq!(instances.len(), 1);
//! assert_eq!(instances[0].definition_id, 451);
//! assert_eq!(instances[0].position, [1.0, 2.0, 3.0]);
//! # Ok::<(), vc_world::WorldError>(())
//! ```

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

use thiserror::Error;

/// Fewest fields an IDE object definition can carry: id, model, TXD, object count
/// and draw distance. A `tobj` adds its on/off times and a flag word.
pub const DEFINITION_FIELDS: usize = 5;

/// Fields in an IPL placement.
pub const INSTANCE_FIELDS: usize = 13;

/// What one model id means.
#[derive(Debug, Clone, PartialEq)]
pub struct ObjectDefinition {
    /// Model id, unique across the map.
    pub id: u32,
    /// Model name, which is also the DFF's name in the archive.
    pub model: String,
    /// Texture dictionary name, also an archive entry.
    pub txd: String,
    /// Distance at which the game stops drawing it.
    pub draw_distance: f32,
    /// Object flags, as stored.
    pub flags: u32,
}

/// One placed object.
#[derive(Debug, Clone, PartialEq)]
pub struct Instance {
    /// Id of the definition this placement uses.
    pub definition_id: u32,
    /// Model name as written next to the placement.
    pub model: String,
    /// Interior the object belongs to: 0 is the outside world, and **-1 is the
    /// "no interior" sentinel** one shipped placement uses, which is why the field
    /// is signed.
    pub interior: i32,
    /// World position.
    pub position: [f32; 3],
    /// Per-axis scale.
    pub scale: [f32; 3],
    /// Orientation, as `(x, y, z, w)`.
    pub rotation: [f32; 4],
}

/// Parses the `objs` and `tobj` sections of an IDE file.
///
/// # Errors
/// Returns [`WorldError::FieldCount`] for a definition that does not carry
/// [`DEFINITION_FIELDS`] values and [`WorldError::Number`] for a field that is not one.
pub fn parse_definitions(text: &str) -> Result<Vec<ObjectDefinition>, WorldError> {
    let mut definitions = Vec::new();
    let mut in_section = false;

    for line in text.lines() {
        let trimmed = line.trim();

        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if trimmed.eq_ignore_ascii_case("end") {
            in_section = false;

            continue;
        }

        if !trimmed.contains(',') {
            in_section =
                trimmed.eq_ignore_ascii_case("objs") || trimmed.eq_ignore_ascii_case("tobj");

            continue;
        }

        if in_section {
            definitions.push(parse_definition(trimmed)?);
        }
    }

    Ok(definitions)
}

fn parse_definition(line: &str) -> Result<ObjectDefinition, WorldError> {
    let fields: Vec<&str> = line.split(',').map(str::trim).collect();

    if fields.len() < DEFINITION_FIELDS {
        return Err(WorldError::FieldCount {
            found: fields.len(),
            expected: DEFINITION_FIELDS,
        });
    }

    // The layout is id, model, TXD, object count, draw distance, then whatever the
    // section adds: a `tobj` carries its on and off times before the flags. Reading
    // the draw distance at index 4 and the flags from the end covers both.
    let draw_distance = number::<f32>(fields[4], 4)?;
    // Flags are the last field in both sections: `objs` ends with them, and `tobj`
    // puts its on and off times before them. A line too short to carry a flag word is
    // treated as having none rather than reading the draw distance as flags.
    let flags = match fields.last() {
        Some(field) if fields.len() >= 6 => number::<u32>(field, fields.len() - 1)?,
        _ => 0,
    };

    Ok(ObjectDefinition {
        id: number::<u32>(fields[0], 0)?,
        model: fields[1].to_owned(),
        txd: fields[2].to_owned(),
        draw_distance,
        flags,
    })
}

/// Parses the `inst` sections of an IPL file.
///
/// # Errors
/// Returns [`WorldError::FieldCount`] for a placement that does not carry
/// [`INSTANCE_FIELDS`] values and [`WorldError::Number`] for a field that is not one.
pub fn parse_instances(text: &str) -> Result<Vec<Instance>, WorldError> {
    let mut instances = Vec::new();
    let mut in_section = false;

    for line in text.lines() {
        let trimmed = line.trim();

        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if trimmed.eq_ignore_ascii_case("end") {
            in_section = false;

            continue;
        }

        if !trimmed.contains(',') {
            in_section = trimmed.eq_ignore_ascii_case("inst");

            continue;
        }

        if in_section {
            instances.push(parse_instance(trimmed)?);
        }
    }

    Ok(instances)
}

fn parse_instance(line: &str) -> Result<Instance, WorldError> {
    let fields: Vec<&str> = line.split(',').map(str::trim).collect();

    if fields.len() < INSTANCE_FIELDS {
        return Err(WorldError::FieldCount {
            found: fields.len(),
            expected: INSTANCE_FIELDS,
        });
    }

    Ok(Instance {
        definition_id: number::<u32>(fields[0], 0)?,
        model: fields[1].to_owned(),
        interior: number::<i32>(fields[2], 2)?,
        position: [
            number::<f32>(fields[3], 3)?,
            number::<f32>(fields[4], 4)?,
            number::<f32>(fields[5], 5)?,
        ],
        scale: [
            number::<f32>(fields[6], 6)?,
            number::<f32>(fields[7], 7)?,
            number::<f32>(fields[8], 8)?,
        ],
        rotation: [
            number::<f32>(fields[9], 9)?,
            number::<f32>(fields[10], 10)?,
            number::<f32>(fields[11], 11)?,
            number::<f32>(fields[12], 12)?,
        ],
    })
}

fn number<T: std::str::FromStr>(field: &str, index: usize) -> Result<T, WorldError> {
    field.parse::<T>().map_err(|_| WorldError::Number { index })
}

/// The map: definitions plus the placements that use them.
#[derive(Debug, Clone, Default)]
pub struct World {
    /// Every object definition.
    pub definitions: Vec<ObjectDefinition>,
    /// Every placement.
    pub instances: Vec<Instance>,
}

impl World {
    /// Whether every placement names a definition that exists.
    pub fn unresolved_instances(&self) -> usize {
        let known: std::collections::HashSet<u32> =
            self.definitions.iter().map(|found| found.id).collect();

        self.instances
            .iter()
            .filter(|instance| !known.contains(&instance.definition_id))
            .count()
    }

    /// Definitions the map never places.
    pub fn unplaced_definitions(&self) -> usize {
        let placed: std::collections::HashSet<u32> = self
            .instances
            .iter()
            .map(|found| found.definition_id)
            .collect();

        self.definitions
            .iter()
            .filter(|definition| !placed.contains(&definition.id))
            .count()
    }
}

/// Side of one streaming cell, in world units. Vice City's draw distances run
/// 150 to 300 units, so a cell that size is a natural streaming granularity.
pub const CELL_SIZE: f32 = 200.0;

/// Which cell a position falls in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CellKey {
    /// Cell index along `x`.
    pub x: i32,
    /// Cell index along `y`.
    pub y: i32,
}

impl CellKey {
    /// The cell a world position falls in.
    pub fn of(position: [f32; 3], size: f32) -> Self {
        Self {
            x: (position[0] / size).floor() as i32,
            y: (position[1] / size).floor() as i32,
        }
    }

    /// Centre of the cell.
    pub fn centre(self, size: f32) -> [f32; 3] {
        [
            (self.x as f32 + 0.5) * size,
            (self.y as f32 + 0.5) * size,
            0.0,
        ]
    }
}

/// Placements bucketed by cell, so streaming can ask for what is nearby.
#[derive(Debug, Clone, Default)]
pub struct CellGrid {
    size: f32,
    cells: std::collections::HashMap<CellKey, Vec<u32>>,
}

impl CellGrid {
    /// Buckets every placement in `world`.
    pub fn build(world: &World, size: f32) -> Self {
        let mut cells: std::collections::HashMap<CellKey, Vec<u32>> =
            std::collections::HashMap::new();

        for (index, instance) in world.instances.iter().enumerate() {
            let key = CellKey::of(instance.position, size);
            cells.entry(key).or_default().push(index as u32);
        }

        Self { size, cells }
    }

    /// Cell side in world units.
    pub const fn size(&self) -> f32 {
        self.size
    }

    /// Placements in a cell.
    pub fn instances(&self, key: CellKey) -> &[u32] {
        self.cells.get(&key).map_or(&[], Vec::as_slice)
    }

    /// How many cells hold placements.
    pub fn cell_count(&self) -> usize {
        self.cells.len()
    }

    /// The most densely placed cell, which is a reasonable place to start.
    ///
    /// Ties break on the key so the start point is the same on every run.
    pub fn busiest(&self) -> Option<CellKey> {
        self.cells
            .iter()
            .max_by_key(|(key, instances)| (instances.len(), std::cmp::Reverse(**key)))
            .map(|(key, _)| *key)
    }

    /// Cells within `radius` of `centre`, nearest first.
    pub fn within(&self, centre: CellKey, radius: i32) -> Vec<CellKey> {
        let mut keys = Vec::new();

        for dx in -radius..=radius {
            for dy in -radius..=radius {
                let key = CellKey {
                    x: centre.x + dx,
                    y: centre.y + dy,
                };

                if self.cells.contains_key(&key) {
                    keys.push(key);
                }
            }
        }

        keys.sort_by_key(|key| ((key.x - centre.x).pow(2) + (key.y - centre.y).pow(2), *key));

        keys
    }
}

/// Failures while reading definitions or placements.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum WorldError {
    /// A line does not carry the expected number of values.
    #[error("line has {found} values, expected {expected}")]
    FieldCount {
        /// Values found.
        found: usize,
        /// Values expected.
        expected: usize,
    },
    /// A value is not a number.
    #[error("field {index} is not a number")]
    Number {
        /// Field index.
        index: usize,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_an_object_definition() {
        let text = "objs\n1210, ap_tower, ap_tower, 1, 299, 0\nend\n";

        let definitions = parse_definitions(text).unwrap();

        assert_eq!(definitions.len(), 1);
        assert_eq!(definitions[0].id, 1210);
        assert_eq!(definitions[0].model, "ap_tower");
        assert_eq!(definitions[0].draw_distance, 299.0);
    }

    #[test]
    fn reads_a_time_controlled_object_with_its_extra_fields() {
        let text = "tobj\n417, lamppost2, dynsigns, 3, 78, 200, 60, 128\nend\n";

        let definitions = parse_definitions(text).unwrap();

        assert_eq!(definitions[0].model, "lamppost2");
        assert_eq!(definitions[0].draw_distance, 78.0);
        assert_eq!(definitions[0].flags, 128);
    }

    #[test]
    fn reads_a_placement_with_scale_and_quaternion() {
        let text = "inst\n465, kb_planter, 0, 8.2, -947.5, 15.7, 1, 1, 1, 0, 0, 0.6, 0.8\nend\n";

        let instances = parse_instances(text).unwrap();
        let instance = &instances[0];

        assert_eq!(instance.definition_id, 465);
        assert_eq!(instance.position, [8.2, -947.5, 15.7]);
        assert_eq!(instance.scale, [1.0, 1.0, 1.0]);
        assert_eq!(instance.rotation, [0.0, 0.0, 0.6, 0.8]);
    }

    #[test]
    fn rejects_a_placement_missing_its_quaternion() {
        let text = "inst\n465, kb_planter, 0, 8.2, -947.5, 15.7, 1, 1, 1, 0, 0, 1\nend\n";

        let result = parse_instances(text);

        assert!(matches!(
            result,
            Err(WorldError::FieldCount { found: 12, .. })
        ));
    }

    #[test]
    fn reads_the_negative_interior_sentinel() {
        let text = "inst\n451, palm, -1, 1, 2, 3, 1, 1, 1, 0, 0, 0, 1\nend\n";

        let instances = parse_instances(text).unwrap();

        assert_eq!(instances[0].interior, -1);
    }

    #[test]
    fn ignores_lines_outside_the_sections() {
        let text = "cull\n1, 2, 3\nend\ninst\n451, palm, 0, 1, 2, 3, 1, 1, 1, 0, 0, 0, 1\nend\n";

        let instances = parse_instances(text).unwrap();

        assert_eq!(instances.len(), 1);
    }

    #[test]
    fn buckets_placements_into_cells() {
        let world = World {
            definitions: Vec::new(),
            instances: vec![
                Instance {
                    definition_id: 1,
                    model: "a".to_owned(),
                    interior: 0,
                    position: [10.0, 10.0, 0.0],
                    scale: [1.0; 3],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                },
                Instance {
                    definition_id: 1,
                    model: "a".to_owned(),
                    interior: 0,
                    position: [30.0, 10.0, 0.0],
                    scale: [1.0; 3],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                },
                Instance {
                    definition_id: 1,
                    model: "a".to_owned(),
                    interior: 0,
                    position: [210.0, 10.0, 0.0],
                    scale: [1.0; 3],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                },
            ],
        };

        let grid = CellGrid::build(&world, 200.0);

        assert_eq!(grid.cell_count(), 2);
        assert_eq!(grid.instances(CellKey { x: 0, y: 0 }).len(), 2);
        assert_eq!(grid.instances(CellKey { x: 1, y: 0 }).len(), 1);
        assert_eq!(grid.busiest(), Some(CellKey { x: 0, y: 0 }));
        assert_eq!(grid.within(CellKey { x: 0, y: 0 }, 1).len(), 2);
    }

    #[test]
    fn counts_placements_that_name_no_definition() {
        let world = World {
            definitions: vec![ObjectDefinition {
                id: 1,
                model: "a".to_owned(),
                txd: "a".to_owned(),
                draw_distance: 100.0,
                flags: 0,
            }],
            instances: vec![
                Instance {
                    definition_id: 1,
                    model: "a".to_owned(),
                    interior: 0,
                    position: [0.0; 3],
                    scale: [1.0; 3],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                },
                Instance {
                    definition_id: 99,
                    model: "missing".to_owned(),
                    interior: 0,
                    position: [0.0; 3],
                    scale: [1.0; 3],
                    rotation: [0.0, 0.0, 0.0, 1.0],
                },
            ],
        };

        assert_eq!(world.unresolved_instances(), 1);
        assert_eq!(world.unplaced_definitions(), 0);
    }
}
