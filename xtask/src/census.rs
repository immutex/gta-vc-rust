//! Installation census: tallies the shipped assets and checks them against `PLAN.md`.
//!
//! Geometry totals are counted over unique model names. The archive contains a
//! handful of duplicate names, so entry counts and model counts differ on
//! purpose; both are reported.

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use vc_transport::{GameRoot, ImageArchive};

/// Totals measured on 2026-10-26 and recorded in `PLAN.md` section 4.
mod expected {
    pub(super) const IMG_ENTRIES: usize = 6043;
    pub(super) const DFF_ENTRIES: u32 = 4617;
    pub(super) const TXD_ENTRIES: u32 = 1368;
    pub(super) const COL_ENTRIES: u32 = 30;
    pub(super) const IFP_ENTRIES: u32 = 28;
    pub(super) const DUPLICATE_NAMES: usize = 11;

    pub(super) const MODELS: u64 = 4613;
    pub(super) const GEOMETRIES: u64 = 6505;
    pub(super) const TRIANGLES: u64 = 2_207_246;
    pub(super) const VERTICES: u64 = 2_818_890;
    pub(super) const DFF_FAILURES: u32 = 0;

    pub(super) const TXD_DICTIONARIES: u32 = 1361;
    pub(super) const TXD_TEXTURES: u64 = 11_997;
    pub(super) const TXD_DXT1_SHAPED: u64 = 10_824;

    // One file, `models/coll/peds.col`, is a known gap: its body layout does not
    // match the map collision layout yet (PLAN.md open probe). Everything else
    // parses exactly, so a second failure is a regression.
    pub(super) const COL_FILES: usize = 34;
    pub(super) const COL_RECORDS: u64 = 4245;
    pub(super) const COL_UNPARSED_FILES: u32 = 1;

    pub(super) const IPL_FILES: usize = 32;
    pub(super) const IPL_INSTANCES: u64 = 9114;
    pub(super) const IPL_CULL_ZONES: u64 = 631;
    pub(super) const IPL_PATH_NODES: u64 = 25_441;

    pub(super) const PATH_BLOCKS: u64 = 1957;
    pub(super) const PATH_POINTS_IN_USE: u64 = 14_732;
    pub(super) const PATH_EMPTY_SLOTS: u64 = 8752;
    pub(super) const PATH_DEAD_LINK_BLOCKS: u64 = 0;

    pub(super) const IDE_FILES: usize = 32;
    pub(super) const IDE_OBJECTS: u64 = 3789;
    pub(super) const IDE_TIME_OBJECTS: u64 = 342;
    pub(super) const IDE_EFFECTS: u64 = 1069;
}

/// Records a mismatch when `found` differs from `want`.
fn check(mismatches: &mut Vec<String>, what: &str, found: u64, want: u64) {
    if found != want {
        mismatches.push(format!("{what}: found {found}, expected {want}"));
    }
}

pub(crate) fn run(verify: bool, verbose: bool) -> Result<()> {
    let root = GameRoot::from_env()?;
    let report = collect(&root, verbose)?;

    report.print();

    if verify {
        report.verify()?;
        println!("\nverify: installation matches PLAN.md section 4");
    }

    Ok(())
}

struct Report {
    install: PathBuf,
    img_entries: usize,
    duplicate_names: Vec<String>,
    by_extension: BTreeMap<String, Totals>,
    models: u64,
    geometries: u64,
    geometries_declared: u64,
    geometry_count_mismatches: u32,
    triangles: u64,
    vertices: u64,
    morph_targets: u64,
    dff_failures: u32,
    geometry_layouts_exact: u32,
    geometry_layout_failures: u32,
    prelit_geometries: u32,
    normal_geometries: u32,
    dff_error_samples: Vec<String>,
    dictionaries: u32,
    textures: u64,
    txd_skipped: u32,
    dxt1_shaped: u64,
    non_power_of_two: u64,
    col_files: usize,
    col_records: u64,
    col_spheres: u64,
    col_boxes: u64,
    col_vertices: u64,
    col_faces: u64,
    col_with_tail: u64,
    col_without_bounds: u64,
    col_failures: u32,
    col_error_samples: Vec<String>,
    col_marker_records: u64,
    ipl_files: usize,
    ipl_instances: u64,
    ipl_cull: u64,
    ipl_pickups: u64,
    ipl_path: u64,
    path_blocks: u64,
    path_points: u64,
    path_empty_slots: u64,
    path_broken_blocks: u64,
    path_bounds: Option<([f32; 3], [f32; 3])>,
    path_failures: u32,
    ide_files: usize,
    ide_objects: u64,
    ide_time_objects: u64,
    ide_effects: u64,
    ide_path: u64,
}

#[derive(Default, Clone, Copy)]
struct Totals {
    count: u32,
    bytes: u64,
}

fn collect(root: &GameRoot, verbose: bool) -> Result<Report> {
    let archive = ImageArchive::open(&root.image())?;
    let mut report = Report {
        install: root.path().to_path_buf(),
        img_entries: archive.len(),
        duplicate_names: archive.duplicate_names(),
        by_extension: BTreeMap::new(),
        models: 0,
        geometries: 0,
        geometries_declared: 0,
        geometry_count_mismatches: 0,
        triangles: 0,
        vertices: 0,
        morph_targets: 0,
        dff_failures: 0,
        geometry_layouts_exact: 0,
        geometry_layout_failures: 0,
        prelit_geometries: 0,
        normal_geometries: 0,
        dff_error_samples: Vec::new(),
        dictionaries: 0,
        textures: 0,
        txd_skipped: 0,
        dxt1_shaped: 0,
        non_power_of_two: 0,
        col_files: 0,
        col_records: 0,
        col_spheres: 0,
        col_boxes: 0,
        col_vertices: 0,
        col_faces: 0,
        col_with_tail: 0,
        col_without_bounds: 0,
        col_failures: 0,
        col_error_samples: Vec::new(),
        col_marker_records: 0,
        ipl_files: 0,
        ipl_instances: 0,
        ipl_cull: 0,
        ipl_pickups: 0,
        ipl_path: 0,
        path_blocks: 0,
        path_points: 0,
        path_empty_slots: 0,
        path_broken_blocks: 0,
        path_bounds: None,
        path_failures: 0,
        ide_files: 0,
        ide_objects: 0,
        ide_time_objects: 0,
        ide_effects: 0,
        ide_path: 0,
    };

    tally_archive(&archive, &mut report)?;
    tally_collision_files(&mut report, &root.path().join("data/maps"))?;
    tally_collision_files(&mut report, &root.path().join("models/coll"))?;
    tally_placement(&mut report, &root.path().join("data/maps"), verbose)?;

    Ok(report)
}

fn tally_archive(archive: &ImageArchive, report: &mut Report) -> Result<()> {
    let mut seen: HashSet<String> = HashSet::new();

    for (index, entry) in archive.entries().iter().enumerate() {
        let extension = entry
            .name()
            .rsplit_once('.')
            .map_or_else(|| "none".to_owned(), |(_, ext)| ext.to_ascii_lowercase());

        let totals = report.by_extension.entry(extension.clone()).or_default();
        totals.count += 1;
        totals.bytes += entry.byte_len();

        let first_occurrence = seen.insert(entry.name().to_ascii_lowercase());

        if !first_occurrence {
            continue;
        }

        let data = archive.data(index)?;

        match extension.as_str() {
            "dff" => tally_dff(entry.name(), data, report),
            "txd" => tally_txd(data, report),
            _ => {}
        }
    }

    Ok(())
}

fn tally_dff(name: &str, data: &[u8], report: &mut Report) {
    match vc_dff::summarize(data) {
        Ok(clump) => {
            report.models += 1;
            report.geometries += u64::from(clump.geometries);
            report.geometries_declared += u64::from(clump.declared_geometries);
            report.triangles += clump.triangles;
            report.vertices += clump.vertices;
            report.morph_targets += clump.morph_targets;
            report.geometry_layouts_exact += clump.layouts_exact;
            report.geometry_layout_failures += clump.layout_failures;
            report.prelit_geometries += clump.prelit_geometries;
            report.normal_geometries += clump.normal_geometries;

            if !clump.geometry_count_matches() {
                report.geometry_count_mismatches += 1;
            }
        }
        Err(error) => {
            report.dff_failures += 1;

            if report.dff_error_samples.len() < 8 {
                report.dff_error_samples.push(format!("{name}: {error}"));
            }
        }
    }
}

fn tally_txd(data: &[u8], report: &mut Report) {
    let Ok(dictionary) = vc_txd::Dictionary::parse(data) else {
        report.txd_skipped += 1;

        return;
    };

    report.dictionaries += 1;
    report.textures += dictionary.textures.len() as u64;
    report.txd_skipped += dictionary.skipped;
    report.dxt1_shaped += dictionary.single_dxt1_levels() as u64;
    report.non_power_of_two += dictionary.non_power_of_two() as u64;
}

fn tally_collision_files(report: &mut Report, directory: &Path) -> Result<()> {
    for path in files_with_extension(directory, "col")? {
        let bytes =
            fs::read(&path).with_context(|| format!("failed to read `{}`", path.display()))?;

        report.col_files += 1;

        let collision = match vc_col::Collision::parse(&bytes) {
            Ok(collision) => collision,
            Err(error) => {
                report.col_failures += 1;
                report.col_marker_records += count_markers(&bytes, b"COLL");

                if report.col_error_samples.len() < 4 {
                    let name = path
                        .strip_prefix(&report.install)
                        .unwrap_or(&path)
                        .display()
                        .to_string();
                    report.col_error_samples.push(format!("{name}: {error}"));
                }

                continue;
            }
        };

        let totals = collision.totals();

        report.col_records += collision.records.len() as u64;
        report.col_spheres += totals.spheres;
        report.col_boxes += totals.boxes;
        report.col_vertices += totals.vertices;
        report.col_faces += totals.faces;
        report.col_with_tail += collision.records_with_tail() as u64;
        report.col_without_bounds += collision.records_without_bounds() as u64;
    }

    Ok(())
}

fn tally_placement(report: &mut Report, directory: &Path, verbose: bool) -> Result<()> {
    for path in files_with_extension(directory, "ipl")? {
        let text = read_text(&path)?;
        let stats = vc_ipl::Stats::parse(&text);

        report.ipl_files += 1;
        report.ipl_instances += stats.instances;
        report.ipl_cull += stats.cull_zones;
        report.ipl_pickups += stats.pickups;
        report.ipl_path += stats.path_nodes;

        tally_path_graph(report, &text);

        if verbose {
            println!(
                "ipl {:<44} inst={:<6} cull={:<5} pick={:<4} path={:<6} blocks={}",
                file_label(report, &path),
                stats.instances,
                stats.cull_zones,
                stats.pickups,
                stats.path_nodes,
                stats.sections
            );
        }
    }

    for path in files_with_extension(directory, "ide")? {
        let text = read_text(&path)?;
        let stats = vc_ide::Stats::parse(&text);

        report.ide_files += 1;
        report.ide_objects += stats.objects;
        report.ide_time_objects += stats.time_objects;
        report.ide_effects += stats.effects;
        report.ide_path += stats.path_nodes;

        if verbose {
            println!(
                "ide {:<44} objs={:<6} tobj={:<5} 2dfx={:<5} path={:<5} blocks={}",
                file_label(report, &path),
                stats.objects,
                stats.time_objects,
                stats.effects,
                stats.path_nodes,
                stats.sections
            );
        }
    }

    Ok(())
}

/// Accumulates the decoded path graph from one IPL file.
fn tally_path_graph(report: &mut Report, text: &str) {
    let Ok(paths) = vc_paths::Paths::parse(text) else {
        report.path_failures += 1;

        return;
    };

    report.path_blocks += paths.blocks.len() as u64;
    report.path_points += paths.points_in_use() as u64;
    report.path_empty_slots += paths.empty_slots() as u64;
    report.path_broken_blocks += paths.blocks_with_dead_links() as u64;

    if let Some((min, max)) = paths.bounds_in_metres() {
        let (current_min, current_max) = report.path_bounds.unwrap_or((min, max));

        report.path_bounds = Some((
            [0, 1, 2].map(|axis| current_min[axis].min(min[axis])),
            [0, 1, 2].map(|axis| current_max[axis].max(max[axis])),
        ));
    }
}

fn file_label(report: &Report, path: &Path) -> String {
    path.strip_prefix(&report.install)
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Number of times `marker` occurs in `bytes`.
///
/// Used to keep a record count meaningful for a file whose body layout is not
/// understood yet, so the total stays honest while the file is reported.
fn count_markers(bytes: &[u8], marker: &[u8]) -> u64 {
    if marker.is_empty() || bytes.len() < marker.len() {
        return 0;
    }

    bytes
        .windows(marker.len())
        .filter(|window| *window == marker)
        .count() as u64
}

fn read_text(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("failed to read `{}`", path.display()))?;

    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn files_with_extension(root: &Path, extension: &str) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];

    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };

        for entry in entries {
            let path = entry?.path();

            if path.is_dir() {
                pending.push(path);
            } else if path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case(extension))
            {
                found.push(path);
            }
        }
    }

    found.sort();

    Ok(found)
}

impl Report {
    /// Number of archive entries carrying `extension`.
    fn entries_with(&self, extension: &str) -> u64 {
        self.by_extension
            .get(extension)
            .map_or(0, |totals| u64::from(totals.count))
    }

    fn print(&self) {
        self.print_archive();
        self.print_models();
        self.print_textures();
        self.print_collision();
        self.print_placement();
    }

    fn print_archive(&self) {
        println!("install      {}", self.install.display());
        println!("img entries  {}", self.img_entries);
        println!(
            "duplicates   {} {:?}",
            self.duplicate_names.len(),
            self.duplicate_names
        );

        println!("\narchive by extension");
        for (extension, totals) in &self.by_extension {
            println!(
                "  {extension:<5} {:>6} entries  {:>10} bytes",
                totals.count, totals.bytes
            );
        }
    }

    fn print_models(&self) {
        println!("\nmodels");
        println!("  models            {}", self.models);
        println!("  geometries        {}", self.geometries);
        println!("  declared          {}", self.geometries_declared);
        println!("  count mismatches  {}", self.geometry_count_mismatches);
        println!("  triangles         {}", self.triangles);
        println!("  vertices          {}", self.vertices);
        println!("  morph targets     {}", self.morph_targets);
        println!("  layouts exact     {}", self.geometry_layouts_exact);
        println!("  layout failures   {}", self.geometry_layout_failures);
        println!("  prelit            {}", self.prelit_geometries);
        println!("  with normals      {}", self.normal_geometries);
        println!("  dff parse errors  {}", self.dff_failures);

        for sample in &self.dff_error_samples {
            println!("    {sample}");
        }
    }

    fn print_textures(&self) {
        println!("\ntextures");
        println!("  dictionaries      {}", self.dictionaries);
        println!("  textures          {}", self.textures);
        println!("  single DXT1 level {}", self.dxt1_shaped);
        println!("  non power of two  {}", self.non_power_of_two);
        println!("  skipped           {}", self.txd_skipped);
    }

    fn print_collision(&self) {
        println!("\ncollision");
        println!("  files             {}", self.col_files);
        println!("  records           {}", self.col_records);
        println!("  spheres           {}", self.col_spheres);
        println!("  boxes             {}", self.col_boxes);
        println!("  vertices          {}", self.col_vertices);
        println!("  faces             {}", self.col_faces);
        println!("  records w/ tail   {}", self.col_with_tail);
        println!("  records no bounds {}", self.col_without_bounds);
        println!("  records by marker {}", self.col_marker_records);
        println!("  parse errors      {}", self.col_failures);

        for sample in &self.col_error_samples {
            println!("    {sample}");
        }
    }

    fn print_placement(&self) {
        println!("\nplacement");
        println!("  ipl files         {}", self.ipl_files);
        println!("  instances         {}", self.ipl_instances);
        println!("  cull zones        {}", self.ipl_cull);
        println!("  pickups           {}", self.ipl_pickups);
        println!("  path lines        {}", self.ipl_path);
        println!("  path blocks       {}", self.path_blocks);
        println!("  points in use     {}", self.path_points);
        println!("  empty slots       {}", self.path_empty_slots);
        println!("  blocks w/ dead    {}", self.path_broken_blocks);
        println!("  path failures     {}", self.path_failures);

        if let Some((min, max)) = self.path_bounds {
            let [min_x, min_y, min_z] = min;
            let [max_x, max_y, max_z] = max;

            println!(
                "  graph bounds m    x[{min_x:.0},{max_x:.0}] y[{min_y:.0},{max_y:.0}] z[{min_z:.0},{max_z:.0}]"
            );
        }
        println!("  ide files         {}", self.ide_files);
        println!("  objects           {}", self.ide_objects);
        println!("  time objects      {}", self.ide_time_objects);
        println!("  2dfx entries      {}", self.ide_effects);
        println!("  path nodes        {}", self.ide_path);
    }

    #[expect(
        clippy::too_many_lines,
        reason = "a flat table of one assertion per measured total; splitting it would only relocate the data"
    )]
    fn expected_checks(&self) -> [(&'static str, u64, u64); 33] {
        [
            (
                "img entries",
                self.img_entries as u64,
                expected::IMG_ENTRIES as u64,
            ),
            (
                "dff entries",
                self.entries_with("dff"),
                u64::from(expected::DFF_ENTRIES),
            ),
            (
                "txd entries",
                self.entries_with("txd"),
                u64::from(expected::TXD_ENTRIES),
            ),
            (
                "col entries",
                self.entries_with("col"),
                u64::from(expected::COL_ENTRIES),
            ),
            (
                "ifp entries",
                self.entries_with("ifp"),
                u64::from(expected::IFP_ENTRIES),
            ),
            (
                "duplicate names",
                self.duplicate_names.len() as u64,
                expected::DUPLICATE_NAMES as u64,
            ),
            ("models", self.models, expected::MODELS),
            ("geometries", self.geometries, expected::GEOMETRIES),
            (
                "declared geometries",
                self.geometries_declared,
                expected::GEOMETRIES,
            ),
            ("triangles", self.triangles, expected::TRIANGLES),
            ("vertices", self.vertices, expected::VERTICES),
            (
                "dff parse errors",
                u64::from(self.dff_failures),
                u64::from(expected::DFF_FAILURES),
            ),
            (
                "geometry layouts exact",
                u64::from(self.geometry_layouts_exact),
                expected::GEOMETRIES,
            ),
            (
                "geometry layout failures",
                u64::from(self.geometry_layout_failures),
                0,
            ),
            (
                "txd dictionaries",
                u64::from(self.dictionaries),
                u64::from(expected::TXD_DICTIONARIES),
            ),
            ("txd textures", self.textures, expected::TXD_TEXTURES),
            (
                "txd dxt1 shaped",
                self.dxt1_shaped,
                expected::TXD_DXT1_SHAPED,
            ),
            (
                "col files",
                self.col_files as u64,
                expected::COL_FILES as u64,
            ),
            ("col records", self.col_records, expected::COL_RECORDS),
            (
                "col unparsed files",
                u64::from(self.col_failures),
                u64::from(expected::COL_UNPARSED_FILES),
            ),
            (
                "ipl files",
                self.ipl_files as u64,
                expected::IPL_FILES as u64,
            ),
            ("ipl instances", self.ipl_instances, expected::IPL_INSTANCES),
            ("ipl cull zones", self.ipl_cull, expected::IPL_CULL_ZONES),
            ("ipl path lines", self.ipl_path, expected::IPL_PATH_NODES),
            ("path blocks", self.path_blocks, expected::PATH_BLOCKS),
            (
                "path points in use",
                self.path_points,
                expected::PATH_POINTS_IN_USE,
            ),
            (
                "path empty slots",
                self.path_empty_slots,
                expected::PATH_EMPTY_SLOTS,
            ),
            (
                "path blocks with dead links",
                self.path_broken_blocks,
                expected::PATH_DEAD_LINK_BLOCKS,
            ),
            ("path parse failures", u64::from(self.path_failures), 0),
            (
                "ide files",
                self.ide_files as u64,
                expected::IDE_FILES as u64,
            ),
            ("ide objects", self.ide_objects, expected::IDE_OBJECTS),
            (
                "ide time objects",
                self.ide_time_objects,
                expected::IDE_TIME_OBJECTS,
            ),
            ("ide effects", self.ide_effects, expected::IDE_EFFECTS),
        ]
    }

    fn verify(&self) -> Result<()> {
        let mut mismatches: Vec<String> = Vec::new();

        for (what, found, want) in self.expected_checks() {
            check(&mut mismatches, what, found, want);
        }

        if self.geometry_count_mismatches != 0 {
            mismatches.push(format!(
                "geometry count mismatches: {}",
                self.geometry_count_mismatches
            ));
        }

        if self.txd_skipped != 0 {
            mismatches.push(format!(
                "txd entries that did not parse: {}",
                self.txd_skipped
            ));
        }

        if mismatches.is_empty() {
            return Ok(());
        }

        let details = mismatches.join("\n  ");

        bail!("installation does not match PLAN.md section 4:\n  {details}")
    }
}
