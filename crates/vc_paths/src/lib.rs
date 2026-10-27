//! Reader for the Vice City path graph.
//!
//! `data/maps/paths.ipl` is loaded by the game (`data/gta_vc.dat` references it) and holds the
//! pedestrian and vehicle navigation graph. Its `path` section is a flat list of blocks, each one
//! introduced by an unindented `<kind>, -1` line and followed by **exactly twelve** tab-indented
//! point records. Each point links to the next point *within its own block*: field 1 is an index in
//! `0..12`, or `-1` for an unused slot.
//!
//! Positions are fixed point, not metres: the file's own values are far larger than the map, and
//! dividing by [`UNITS_PER_METRE`] puts every used point inside the city with a plausible elevation.
//! Both the scale and the meaning of several fields are working hypotheses — see `PLAN.md` §15.2 —
//! so the raw values are kept alongside the derived position rather than replaced by it.
//!
//! # Examples
//!
//! ```
//! use vc_paths::Paths;
//!
//! let text = "path\n1, -1\n\t2, 1, 0, 160, 320, 160, 2, 1, 1, 1, 0, 1\n".to_string()
//!     + &"\t0, -1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1\n".repeat(11)
//!     + "end\n";
//!
//! let paths = Paths::parse(&text)?;
//!
//! assert_eq!(paths.blocks.len(), 1);
//! assert_eq!(paths.points_in_use(), 1);
//! # Ok::<(), vc_paths::PathError>(())
//! ```

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

use thiserror::Error;

/// Points in every block, as shipped.
pub const POINTS_PER_BLOCK: usize = 12;

/// Fixed-point units per metre.
///
/// Working hypothesis, not a decoded fact: at this scale every used point lands inside the city
/// (x within ±2,100 and y within ±2,100 of the world box derived from placed instances) with a
/// median elevation of ~10 m, and a signed 16-bit value with four fractional bits covers exactly
/// ±2,048 m — the extent of the map. Treat a change here as a behaviour change, not a tweak.
pub const UNITS_PER_METRE: f32 = 16.0;

/// One point of a path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathPoint {
    /// Field 0: point kind. **Zero means the slot is padding**, and that is the marker to test:
    /// every padding slot also carries a zero position, and 1 and 2 are the kinds in use.
    pub kind: u8,
    /// Field 1: index of the next point *within this block*, or `None`.
    ///
    /// `None` on a point in use means the end of a chain, not an empty slot.
    pub next: Option<u8>,
    /// Fields 3 to 5: position in fixed-point units, as stored.
    pub raw_position: [f32; 3],
    /// Field 2: not decoded, always 0 or 1 in shipped data.
    pub field2: u8,
    /// Field 6: not decoded, 0 to 200 with a median of 0.
    pub field6: u8,
    /// Fields 7 to 10: not decoded, small values.
    pub field7_to_10: [u8; 4],
    /// Field 11: not decoded, 0.2 to 1.0.
    pub field11: f32,
}

impl PathPoint {
    /// Whether this slot holds a point rather than padding.
    pub const fn is_in_use(&self) -> bool {
        self.kind != 0
    }

    /// Whether this point ends a chain.
    pub const fn is_chain_end(&self) -> bool {
        self.is_in_use() && self.next.is_none()
    }

    /// Position in metres, converted at [`UNITS_PER_METRE`].
    pub fn position(&self) -> [f32; 3] {
        self.raw_position.map(|value| value / UNITS_PER_METRE)
    }
}

/// One block: a kind and exactly [`POINTS_PER_BLOCK`] points.
#[derive(Debug, Clone, PartialEq)]
pub struct PathBlock {
    /// Block kind from the header: 0, 1 or 2 in shipped data.
    pub kind: u8,
    /// The block's points, in file order.
    pub points: Vec<PathPoint>,
}

/// The parsed graph.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Paths {
    /// Blocks in file order.
    pub blocks: Vec<PathBlock>,
}

impl Paths {
    /// Parses the `path` section of an IPL file, ignoring every other section.
    ///
    /// # Errors
    /// Returns [`PathError::PointCount`] when a block does not hold exactly twelve points, and
    /// [`PathError::FieldCount`] when a point record does not have twelve fields.
    pub fn parse(text: &str) -> Result<Self, PathError> {
        let mut paths = Self::default();
        let mut in_path = false;
        let mut block: Option<PathBlock> = None;

        for line in text.lines() {
            let trimmed = line.trim();

            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            let indented = line.starts_with('\t') || line.starts_with(' ');

            if indented {
                if let Some(current) = block.as_mut() {
                    current.points.push(parse_point(trimmed)?);
                }

                continue;
            }

            paths.close(&mut block, paths.blocks.len())?;

            if trimmed.eq_ignore_ascii_case("end") {
                in_path = false;

                continue;
            }

            if trimmed.eq_ignore_ascii_case("path") {
                in_path = true;

                continue;
            }

            // Inside the path section, an unindented line is a block header.
            if in_path {
                block = Some(PathBlock {
                    kind: parse_kind(trimmed)?,
                    points: Vec::with_capacity(POINTS_PER_BLOCK),
                });
            }
        }

        paths.close(&mut block, paths.blocks.len())?;

        Ok(paths)
    }

    fn close(&mut self, block: &mut Option<PathBlock>, index: usize) -> Result<(), PathError> {
        let Some(finished) = block.take() else {
            return Ok(());
        };

        if finished.points.len() != POINTS_PER_BLOCK {
            return Err(PathError::PointCount {
                block: index,
                found: finished.points.len(),
            });
        }

        self.blocks.push(finished);

        Ok(())
    }

    /// Points that hold data, across every block.
    pub fn points_in_use(&self) -> usize {
        self.blocks
            .iter()
            .flat_map(|block| &block.points)
            .filter(|point| point.is_in_use())
            .count()
    }

    /// Slots that are padding.
    pub fn empty_slots(&self) -> usize {
        self.blocks
            .iter()
            .flat_map(|block| &block.points)
            .filter(|point| !point.is_in_use())
            .count()
    }

    /// Points that end a chain, having no next point.
    pub fn chain_ends(&self) -> usize {
        self.blocks
            .iter()
            .flat_map(|block| &block.points)
            .filter(|point| point.is_chain_end())
            .count()
    }

    /// Blocks holding a point whose next point is padding.
    pub fn blocks_with_dead_links(&self) -> usize {
        self.blocks
            .iter()
            .filter(|block| {
                block
                    .points
                    .iter()
                    .filter_map(|point| point.next)
                    .any(|next| {
                        block
                            .points
                            .get(next as usize)
                            .is_none_or(|target| !target.is_in_use())
                    })
            })
            .count()
    }

    /// Bounding box of the used points, in metres, as `[min, max]`.
    pub fn bounds_in_metres(&self) -> Option<([f32; 3], [f32; 3])> {
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];
        let mut any = false;

        for point in self.blocks.iter().flat_map(|block| &block.points) {
            if !point.is_in_use() {
                continue;
            }

            any = true;

            for axis in 0..3 {
                min[axis] = min[axis].min(point.position()[axis]);
                max[axis] = max[axis].max(point.position()[axis]);
            }
        }

        any.then_some((min, max))
    }
}

fn parse_kind(header: &str) -> Result<u8, PathError> {
    header
        .split(',')
        .next()
        .and_then(|field| field.trim().parse::<u8>().ok())
        .ok_or(PathError::FieldCount { found: 0 })
}

fn parse_point(record: &str) -> Result<PathPoint, PathError> {
    let fields: Vec<&str> = record.split(',').map(str::trim).collect();

    if fields.len() != 12 {
        return Err(PathError::FieldCount {
            found: fields.len(),
        });
    }

    let number = |index: usize| -> Result<f32, PathError> {
        fields[index]
            .parse::<f32>()
            .map_err(|_| PathError::FieldCount {
                found: fields.len(),
            })
    };

    let next = number(1)?;

    Ok(PathPoint {
        kind: number(0)? as u8,
        next: (next >= 0.0).then_some(next as u8),
        raw_position: [number(3)?, number(4)?, number(5)?],
        field2: number(2)? as u8,
        field6: number(6)? as u8,
        field7_to_10: [
            number(7)? as u8,
            number(8)? as u8,
            number(9)? as u8,
            number(10)? as u8,
        ],
        field11: number(11)?,
    })
}

/// Failures while reading a path graph.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum PathError {
    /// A block does not hold exactly twelve points.
    #[error("block {block} holds {found} points, expected {POINTS_PER_BLOCK}")]
    PointCount {
        /// Index of the offending block.
        block: usize,
        /// Points found.
        found: usize,
    },
    /// A record does not have twelve fields.
    #[error("path record has {found} fields, expected 12")]
    FieldCount {
        /// Fields found.
        found: usize,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block(kind: u8, points: &[&str]) -> String {
        let mut text = format!("{kind}, -1\n");

        for point in points {
            text.push('\t');
            text.push_str(point);
            text.push('\n');
        }

        for _ in points.len()..POINTS_PER_BLOCK {
            text.push_str("\t0, -1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1\n");
        }

        text
    }

    fn document(blocks: &[String]) -> String {
        format!("# comment\ninst\nend\npath\n{}end\n", blocks.join(""))
    }

    #[test]
    fn accepts_an_empty_path_section() {
        let text = "inst\nend\npath\nend\n";

        let paths = Paths::parse(text).unwrap();

        assert_eq!(paths.blocks.len(), 0);
    }

    #[test]
    fn does_not_treat_the_keyword_as_a_block_header() {
        let text = "path\n1, -1\n\t2, 1, 0, 1, 2, 3, 2, 1, 1, 1, 0, 1\n".to_string()
            + &"\t0, -1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1\n".repeat(11)
            + "end\n";

        let paths = Paths::parse(&text).unwrap();

        assert_eq!(paths.blocks.len(), 1);
        assert_eq!(paths.blocks[0].kind, 1);
    }

    #[test]
    fn reads_one_block_with_twelve_points() {
        let text = document(&[block(1, &["2, 1, 0, 160, 320, 160, 2, 1, 1, 1, 0, 1"])]);

        let paths = Paths::parse(&text).unwrap();

        assert_eq!(paths.blocks.len(), 1);
        assert_eq!(paths.blocks[0].points.len(), POINTS_PER_BLOCK);
        assert_eq!(paths.blocks[0].kind, 1);
    }

    #[test]
    fn reads_two_blocks() {
        let text = document(&[block(0, &[]), block(2, &[])]);

        let paths = Paths::parse(&text).unwrap();

        assert_eq!(paths.blocks.len(), 2);
        assert_eq!(paths.blocks[1].kind, 2);
    }

    #[test]
    fn treats_kind_zero_as_padding() {
        let text = document(&[block(1, &["0, -1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1"])]);

        let paths = Paths::parse(&text).unwrap();

        assert!(!paths.blocks[0].points[0].is_in_use());
        assert_eq!(paths.points_in_use(), 0);
    }

    #[test]
    fn treats_a_point_with_no_next_as_a_chain_end() {
        let text = document(&[block(1, &["2, -1, 0, 160, 320, 160, 2, 1, 1, 1, 0, 1"])]);

        let paths = Paths::parse(&text).unwrap();

        assert!(paths.blocks[0].points[0].is_in_use());
        assert_eq!(paths.chain_ends(), 1);
    }

    #[test]
    fn counts_points_in_use_and_empty_slots() {
        let text = document(&[block(
            1,
            &[
                "2, 1, 0, 160, 320, 160, 2, 1, 1, 1, 0, 1",
                "0, -1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1",
            ],
        )]);

        let paths = Paths::parse(&text).unwrap();

        assert_eq!(paths.points_in_use(), 1);
        assert_eq!(paths.empty_slots(), POINTS_PER_BLOCK - 1);
    }

    #[test]
    fn converts_fixed_point_positions_to_metres() {
        let text = document(&[block(1, &["2, 1, 0, -1600, 3200, 160, 2, 1, 1, 1, 0, 1"])]);

        let paths = Paths::parse(&text).unwrap();

        assert_eq!(paths.blocks[0].points[0].position(), [-100.0, 200.0, 10.0]);
    }

    #[test]
    fn reports_bounds_of_used_points_only() {
        let text = document(&[block(
            1,
            &[
                "2, 1, 0, -1600, 3200, 160, 2, 1, 1, 1, 0, 1",
                "0, -1, 0, 99999, 99999, 99999, 2, 1, 1, 1, 0, 1",
            ],
        )]);

        let paths = Paths::parse(&text).unwrap();
        let (min, max) = paths.bounds_in_metres().unwrap();

        assert_eq!(min, [-100.0, 200.0, 10.0]);
        assert_eq!(max, [-100.0, 200.0, 10.0]);
    }

    #[test]
    fn rejects_a_block_that_is_not_twelve_points() {
        let text = "path\n1, -1\n\t2, 1, 0, 1, 2, 3, 2, 1, 1, 1, 0, 1\nend\n";

        let result = Paths::parse(text);

        assert_eq!(
            result.unwrap_err(),
            PathError::PointCount { block: 0, found: 1 }
        );
    }

    #[test]
    fn rejects_a_record_with_the_wrong_field_count() {
        let text = "path\n1, -1\n\t2, 1, 0, 1, 2\nend\n";

        let result = Paths::parse(text);

        assert_eq!(result.unwrap_err(), PathError::FieldCount { found: 5 });
    }

    #[test]
    fn ignores_every_section_other_than_path() {
        let text = "inst\n1, 2, 3\nend\ncull\nend\n";

        let paths = Paths::parse(text).unwrap();

        assert_eq!(paths.blocks.len(), 0);
    }

    #[test]
    fn counts_blocks_that_link_to_padding() {
        let text = document(&[block(1, &["2, 5, 0, 160, 320, 160, 2, 1, 1, 1, 0, 1"])]);

        let paths = Paths::parse(&text).unwrap();

        assert_eq!(paths.blocks_with_dead_links(), 1);
    }
}
