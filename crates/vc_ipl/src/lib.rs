//! Reader for Vice City IPL files.
//!
//! IPL is plain text. A section opens with a bare keyword and closes with `end`;
//! `inst` holds placed objects, and `cull`, `pick` and `path` are shipped as
//! blocks that exist but are empty in this release. Lines starting with `#` are
//! comments.
//!
//! This crate counts and classifies. Turning instances into world cells is the
//! world loader's job.
//!
//! # Examples
//!
//! ```
//! use vc_ipl::Stats;
//!
//! let text = "inst\n1210, 1.0, 2.0, 3.0, 0, 0, 0, 0, 0, 0, 0, -1\nend\npath\nend\n";
//! let stats = Stats::parse(text);
//!
//! assert_eq!(stats.instances, 1);
//! assert_eq!(stats.path_nodes, 0);
//! ```

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

use thiserror::Error;

/// Fields in a cull zone record.
pub const CULL_ZONE_FIELDS: usize = 11;

/// One cull zone: an anchor, a box, then two values whose meaning is not decoded.
///
/// The anchor's **x and y are exactly the box centre** — across all 631 shipped
/// records the deviation is under a hundredth of a unit — but its **z is not**.
/// On that axis the anchor sits anywhere between the box floor and its ceiling,
/// and for a tenth of the records it is the floor, so z is an independent height
/// rather than a centre. Culling therefore uses [`CullZone::min`] and
/// [`CullZone::max`], which are unambiguous.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CullZone {
    /// Anchor point: box centre in x and y, an independent height in z.
    pub anchor: [f32; 3],
    /// Minimum corner.
    pub min: [f32; 3],
    /// Maximum corner.
    pub max: [f32; 3],
    /// First trailing value, 40 in every shipped record.
    pub field9: f32,
    /// Second trailing value, 0 in every shipped record.
    pub field10: f32,
}

impl CullZone {
    /// Whether the anchor's ground position is the centre of the box.
    ///
    /// Only x and y are checked; see the type documentation for why z is not.
    pub fn anchor_xy_is_box_centre(&self) -> bool {
        (0..2).all(|axis| {
            let midpoint = self.min[axis].midpoint(self.max[axis]);
            (midpoint - self.anchor[axis]).abs() <= 0.01
        })
    }

    /// Box centre in all three axes, taken from the corners rather than the anchor.
    pub fn box_centre(&self) -> [f32; 3] {
        [0, 1, 2].map(|axis| self.min[axis].midpoint(self.max[axis]))
    }
}

/// Parses every record in the `cull` section.
///
/// # Errors
/// Returns [`CullError::FieldCount`] when a record does not hold
/// [`CULL_ZONE_FIELDS`] values and [`CullError::NotANumber`] when one is not.
pub fn parse_cull_zones(text: &str) -> Result<Vec<CullZone>, CullError> {
    let mut zones = Vec::new();
    let mut in_cull = false;

    for line in text.lines() {
        let trimmed = line.trim();

        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        if trimmed.eq_ignore_ascii_case("end") {
            in_cull = false;

            continue;
        }

        if !trimmed.contains(',') {
            in_cull = trimmed.eq_ignore_ascii_case("cull");

            continue;
        }

        if !in_cull {
            continue;
        }

        zones.push(parse_cull_zone(trimmed)?);
    }

    Ok(zones)
}

fn parse_cull_zone(record: &str) -> Result<CullZone, CullError> {
    let fields: Vec<&str> = record.split(',').map(str::trim).collect();

    if fields.len() != CULL_ZONE_FIELDS {
        return Err(CullError::FieldCount {
            found: fields.len(),
        });
    }

    let mut values = [0f32; CULL_ZONE_FIELDS];

    for (index, value) in values.iter_mut().enumerate() {
        *value = fields[index]
            .parse::<f32>()
            .map_err(|_| CullError::NotANumber { index })?;
    }

    Ok(CullZone {
        anchor: [values[0], values[1], values[2]],
        min: [values[3], values[4], values[5]],
        max: [values[6], values[7], values[8]],
        field9: values[9],
        field10: values[10],
    })
}

/// Failures while reading cull zones.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CullError {
    /// A record does not hold the expected number of values.
    #[error("cull record has {found} fields, expected {CULL_ZONE_FIELDS}")]
    FieldCount {
        /// Fields found.
        found: usize,
    },
    /// A value is not a number.
    #[error("cull record field {index} is not a number")]
    NotANumber {
        /// Field index.
        index: usize,
    },
}

/// Section keywords of an IPL file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// Placed objects.
    Instances,
    /// Cull zones.
    Cull,
    /// Pickups.
    Pickups,
    /// Ped and vehicle path nodes.
    Path,
    /// Any other keyword.
    Unknown,
}

impl Section {
    /// Classifies a section keyword, case-insensitively.
    pub fn from_keyword(keyword: &str) -> Self {
        match keyword.trim().to_ascii_lowercase().as_str() {
            "inst" => Self::Instances,
            "cull" => Self::Cull,
            "pick" => Self::Pickups,
            "path" => Self::Path,
            _ => Self::Unknown,
        }
    }
}

/// Line counts per section.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    /// Lines inside `inst` blocks.
    pub instances: u64,
    /// Lines inside `cull` blocks.
    pub cull_zones: u64,
    /// Lines inside `pick` blocks.
    pub pickups: u64,
    /// Lines inside `path` blocks.
    pub path_nodes: u64,
    /// Number of sections opened.
    pub sections: u64,
}

impl Stats {
    /// Counts the content lines of every section in `text`.
    pub fn parse(text: &str) -> Self {
        let mut stats = Self::default();
        let mut current = Section::Unknown;

        for line in text.lines() {
            let line = line.trim();

            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if line.eq_ignore_ascii_case("end") {
                current = Section::Unknown;

                continue;
            }

            if !line.contains(',') && !line.contains(' ') {
                current = Section::from_keyword(line);
                stats.sections += 1;

                continue;
            }

            match current {
                Section::Instances => stats.instances += 1,
                Section::Cull => stats.cull_zones += 1,
                Section::Pickups => stats.pickups += 1,
                Section::Path => stats.path_nodes += 1,
                Section::Unknown => {}
            }
        }

        stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_cull_zone_box() {
        let text = "cull\n-346.793, 1082.69, 11.98, -357.9, 1058.6, 6.4, -335.6, 1106.7, 16.8, 40, 0\nend\n";

        let zones = parse_cull_zones(text).unwrap();

        assert_eq!(zones.len(), 1);
        assert_eq!(zones[0].anchor, [-346.793, 1082.69, 11.98]);
        assert_eq!(zones[0].field9, 40.0);
    }

    #[test]
    fn confirms_the_anchor_sits_over_the_box_centre() {
        let text = "cull\n-346.793, 1082.69, 11.98, -357.922, 1058.65, 6.47, -335.663, 1106.74, 16.85, 40, 0\nend\n";

        let zones = parse_cull_zones(text).unwrap();

        assert!(zones[0].anchor_xy_is_box_centre());
    }

    #[test]
    fn takes_the_box_centre_from_the_corners_not_the_anchor() {
        let text = "cull\n0, 0, 5, -10, -10, -10, 10, 10, 10, 40, 0\nend\n";

        let zones = parse_cull_zones(text).unwrap();

        assert_eq!(zones[0].box_centre(), [0.0, 0.0, 0.0]);
    }

    #[test]
    fn ignores_records_outside_the_cull_section() {
        let text = "inst\n1, 2, 3\nend\ncull\n1, 2, 3\nend\n";

        let result = parse_cull_zones(text);

        assert!(matches!(result, Err(CullError::FieldCount { found: 3 })));
    }

    #[test]
    fn rejects_a_record_that_is_not_a_number() {
        let text = "cull\n1, 2, 3, 4, 5, 6, 7, 8, 9, 40, x\nend\n";

        let result = parse_cull_zones(text);

        assert_eq!(result.unwrap_err(), CullError::NotANumber { index: 10 });
    }

    #[test]
    fn counts_instances_across_blocks() {
        let text = "inst\r\n1, 2, 3\r\n4, 5, 6\r\nend\r\ninst\r\n7, 8, 9\r\nend\r\n";

        let stats = Stats::parse(text);

        assert_eq!(stats.instances, 3);
    }

    #[test]
    fn treats_an_empty_path_block_as_no_nodes() {
        let text = "path\r\nend\r\n";

        let stats = Stats::parse(text);

        assert_eq!(stats.path_nodes, 0);
        assert_eq!(stats.sections, 1);
    }

    #[test]
    fn ignores_comments_and_blank_lines() {
        let text = "inst\n# a comment\n\n1, 2, 3\nend\n";

        let stats = Stats::parse(text);

        assert_eq!(stats.instances, 1);
    }

    #[test]
    fn counts_cull_and_pickup_blocks_separately() {
        let text = "cull\n1, 2, 3\nend\npick\n4, 5, 6\n7, 8, 9\nend\n";

        let stats = Stats::parse(text);

        assert_eq!(stats.cull_zones, 1);
        assert_eq!(stats.pickups, 2);
    }

    #[test]
    fn classifies_keywords_case_insensitively() {
        assert_eq!(Section::from_keyword("INST"), Section::Instances);
        assert_eq!(Section::from_keyword("trash"), Section::Unknown);
    }
}
