//! Reader for Vice City IDE files.
//!
//! IDE is plain text with the same block shape as IPL: a bare keyword, entries,
//! then `end`. Sections cover objects (`objs`), time-controlled objects (`tobj`),
//! visual effects (`2dfx`), path nodes (`path`), plus `anim`, `weap`, `cars`,
//! `peds` and `hier`. Lines starting with `#` are comments.
//!
//! # Examples
//!
//! ```
//! use vc_ide::Stats;
//!
//! let text = "objs\n1210, ap_tower, ap_tower, 299, 0\nend\n";
//! let stats = Stats::parse(text);
//!
//! assert_eq!(stats.objects, 1);
//! ```

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

/// Section keywords counted by [`Stats`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// `objs`: plain objects.
    Objects,
    /// `tobj`: time-controlled objects.
    TimeObjects,
    /// `2dfx`: effects, lights and particles attached to objects.
    Effects,
    /// `path`: path nodes.
    Path,
    /// Any other keyword.
    Unknown,
}

impl Section {
    /// Classifies a section keyword, case-insensitively.
    pub fn from_keyword(keyword: &str) -> Self {
        match keyword.trim().to_ascii_lowercase().as_str() {
            "objs" => Self::Objects,
            "tobj" => Self::TimeObjects,
            "2dfx" => Self::Effects,
            "path" => Self::Path,
            _ => Self::Unknown,
        }
    }
}

/// Entry counts per section.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    /// Entries in `objs` blocks.
    pub objects: u64,
    /// Entries in `tobj` blocks.
    pub time_objects: u64,
    /// Entries in `2dfx` blocks.
    pub effects: u64,
    /// Entries in `path` blocks.
    pub path_nodes: u64,
    /// Number of sections opened.
    pub sections: u64,
}

impl Stats {
    /// Counts the entries of every section in `text`.
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

            if !line.contains(',') {
                current = Section::from_keyword(line);
                stats.sections += 1;

                continue;
            }

            match current {
                Section::Objects => stats.objects += 1,
                Section::TimeObjects => stats.time_objects += 1,
                Section::Effects => stats.effects += 1,
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
    fn counts_object_entries() {
        let text = "objs\r\n1210, ap_tower, ap_tower, 299, 0\r\n1211, ap_fence, ap_fence, 299, 0\r\nend\r\n";

        let stats = Stats::parse(text);

        assert_eq!(stats.objects, 2);
    }

    #[test]
    fn counts_effect_entries_with_many_values() {
        let text = "2dfx\n1210, -53.9, -31.1, -6.9, 184, 255, 0, 120, 3, 4\nend\n";

        let stats = Stats::parse(text);

        assert_eq!(stats.effects, 1);
    }

    #[test]
    fn treats_an_empty_path_block_as_no_nodes() {
        let text = "path\nend\n";

        let stats = Stats::parse(text);

        assert_eq!(stats.path_nodes, 0);
    }

    #[test]
    fn ignores_sections_it_does_not_count() {
        let text = "cars\n130, landstal, landstal, car, LANDSTAL, 9, 0, 0, -1\nend\n";

        let stats = Stats::parse(text);

        assert_eq!(stats.objects, 0);
        assert_eq!(stats.sections, 1);
    }

    #[test]
    fn classifies_keywords_case_insensitively() {
        assert_eq!(Section::from_keyword("OBJS"), Section::Objects);
        assert_eq!(Section::from_keyword("2dfx"), Section::Effects);
    }
}
