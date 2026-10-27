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
