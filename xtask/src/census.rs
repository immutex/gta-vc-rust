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
    pub(super) const TXD_BC1: u64 = 10_824;
    pub(super) const TXD_BC23: u64 = 1149;
    pub(super) const TXD_PALETTED: u64 = 24;
    pub(super) const TXD_WITH_MIPS: u64 = 895;

    // Cooked bytes exceed the source by 539,712: paletted rasters expand from
    // indices to RGBA8, which more than pays for the mip length prefixes the
    // cooker strips. Both totals are gated because their difference is a
    // property of those two decisions, not a free number.
    pub(super) const COOKED_BYTES: u64 = 179_522_336;
    pub(super) const SOURCE_LEVEL_BYTES: u64 = 178_982_624;
    pub(super) const TXD_UNCLASSIFIED: u64 = 0;

    // One file, `models/coll/peds.col`, is a known gap: its body layout does not
    // match the map collision layout yet (PLAN.md open probe). Everything else
    // parses exactly, so a second failure is a regression.
    pub(super) const COL_FILES: usize = 34;
    pub(super) const COL_RECORDS: u64 = 4245;
    pub(super) const COL_UNPARSED_FILES: u32 = 1;

    pub(super) const IPL_FILES: usize = 32;
    pub(super) const IPL_INSTANCES: u64 = 9114;
    pub(super) const IPL_CULL_ZONES: u64 = 631;
    pub(super) const CULL_BOXES: u64 = 631;
    // One shipped record places its anchor 1.55 units off the box centre on x
    // while the other 630 are exact to within a hundredth; see PLAN.md section 4.14.
    pub(super) const CULL_CENTRE_MISMATCHES: u64 = 1;
    pub(super) const IPL_PATH_NODES: u64 = 25_441;

    pub(super) const PATH_BLOCKS: u64 = 1957;
    pub(super) const PATH_POINTS_IN_USE: u64 = 14_732;
    pub(super) const PATH_EMPTY_SLOTS: u64 = 8752;
    pub(super) const PATH_DEAD_LINK_BLOCKS: u64 = 0;

    pub(super) const IDE_FILES: usize = 32;
    pub(super) const IDE_OBJECTS: u64 = 3789;
    pub(super) const IDE_TIME_OBJECTS: u64 = 342;
    pub(super) const IDE_EFFECTS: u64 = 1069;
    pub(super) const WORLD_DEFINITIONS: u64 = 4131;
    pub(super) const WORLD_INSTANCES: u64 = 9114;

    pub(super) const ARCHIVE_FINGERPRINT: &str =
        "bf76407576847344e9a005dce07bfc309473ed103c1055be13e52bd7d9d56325";

    pub(super) const TIMECYC_WEATHERS: u64 = 7;
    pub(super) const TIMECYC_HOURS: u64 = 168;
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
    fingerprint: String,
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
    decoded_triangles: u64,
    decoded_vertices: u64,
    decoded_index_errors: u64,
    decoded_non_finite: u64,
    decode_failures: u32,
    material_slots: u64,
    named_slots: u64,
    dff_error_samples: Vec<String>,
    dictionaries: u32,
    textures: u64,
    txd_skipped: u32,
    cooked_textures: u64,
    cooked_bytes: u64,
    source_level_bytes: u64,
    cooked_bc1: u64,
    cooked_bc3: u64,
    cooked_rgba8: u64,
    cook_failures: u32,
    dxt1_shaped: u64,
    bc1_textures: u64,
    bc23_textures: u64,
    paletted_textures: u64,
    unclassified_textures: u64,
    chains_exact: u64,
    textures_with_mips: u64,
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
    cull_zones: u64,
    cull_centre_mismatches: u64,
    cull_failures: u32,
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
    world_definitions: u64,
    world_instances: u64,
    world_unresolved: u64,
    world_missing_models: u64,
    world_missing_txds: u64,
    world_parse_failures: u32,
    world_error_samples: Vec<String>,
    timecyc_weathers: u64,
    timecyc_hours: u64,
    timecyc_far_clip_min: f32,
    timecyc_far_clip_max: f32,
    timecyc_failures: u32,
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
        fingerprint: archive_fingerprint(&archive),
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
        decoded_triangles: 0,
        decoded_vertices: 0,
        decoded_index_errors: 0,
        decoded_non_finite: 0,
        decode_failures: 0,
        material_slots: 0,
        named_slots: 0,
        dff_error_samples: Vec::new(),
        dictionaries: 0,
        textures: 0,
        txd_skipped: 0,
        cooked_textures: 0,
        cooked_bytes: 0,
        source_level_bytes: 0,
        cooked_bc1: 0,
        cooked_bc3: 0,
        cooked_rgba8: 0,
        cook_failures: 0,
        dxt1_shaped: 0,
        bc1_textures: 0,
        bc23_textures: 0,
        paletted_textures: 0,
        unclassified_textures: 0,
        chains_exact: 0,
        textures_with_mips: 0,
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
        cull_zones: 0,
        cull_centre_mismatches: 0,
        cull_failures: 0,
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
        world_definitions: 0,
        world_instances: 0,
        world_unresolved: 0,
        world_missing_models: 0,
        world_missing_txds: 0,
        world_parse_failures: 0,
        world_error_samples: Vec::new(),
        timecyc_weathers: 0,
        timecyc_hours: 0,
        timecyc_far_clip_min: 0.0,
        timecyc_far_clip_max: 0.0,
        timecyc_failures: 0,
    };

    tally_archive(&archive, &mut report)?;
    tally_collision_files(&mut report, &root.path().join("data/maps"))?;
    tally_collision_files(&mut report, &root.path().join("models/coll"))?;
    tally_placement(&mut report, &root.path().join("data/maps"), verbose)?;
    tally_timecycle(&mut report, &root.path().join("data/timecyc.dat"))?;
    tally_world(&mut report, &root.path().join("data/maps"), &archive)?;

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
            report.decoded_triangles += clump.decoded_triangles;
            report.decoded_vertices += clump.decoded_vertices;
            report.decoded_index_errors += clump.decoded_index_errors;
            report.decoded_non_finite += clump.decoded_non_finite;
            report.decode_failures += clump.decode_failures;
            report.material_slots += clump.material_slots;
            report.named_slots += clump.named_slots;
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
    for texture in &dictionary.textures {
        report.source_level_bytes += texture.level_data.len() as u64;

        match vc_cook::cook_texture(texture) {
            Ok(cooked) => {
                report.cooked_textures += 1;
                report.cooked_bytes += cooked.byte_len() as u64;

                match cooked.format {
                    vc_cook::TextureFormat::Bc1 => report.cooked_bc1 += 1,
                    vc_cook::TextureFormat::Bc3 => report.cooked_bc3 += 1,
                    vc_cook::TextureFormat::Rgba8 => report.cooked_rgba8 += 1,
                }
            }
            Err(_) => report.cook_failures += 1,
        }
    }

    report.dxt1_shaped += dictionary.single_dxt1_levels() as u64;
    report.bc1_textures += dictionary.bc1_count() as u64;
    report.bc23_textures += dictionary.bc23_count() as u64;
    report.paletted_textures += dictionary.paletted_count() as u64;
    report.unclassified_textures += dictionary.unclassified_count() as u64;
    report.chains_exact += dictionary.chains_exact() as u64;
    report.textures_with_mips += dictionary.with_mips() as u64;
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
        tally_cull_zones(report, &text);

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

/// Fingerprints the archive directory: names, offsets and lengths, in order.
///
/// This is the install's identity. A different build of the game changes it, and
/// that is the point: the census refuses to measure an install it has not been
/// asked to measure, rather than quietly reporting numbers from another release.
fn archive_fingerprint(archive: &ImageArchive) -> String {
    let mut hasher = blake3::Hasher::new();

    for entry in archive.entries() {
        hasher.update(entry.name().as_bytes());
        hasher.update(&entry.byte_offset().to_le_bytes());
        hasher.update(&entry.byte_len().to_le_bytes());
    }

    hasher.finalize().to_hex().to_string()
}

/// Builds the world from IDE definitions and IPL placements, then checks that every
/// placement resolves to a definition and every definition to archive entries.
fn tally_world(report: &mut Report, directory: &Path, archive: &ImageArchive) -> Result<()> {
    let mut world = vc_world::World::default();

    for path in files_with_extension(directory, "ide")? {
        let text = read_text(&path)?;

        match vc_world::parse_definitions(&text) {
            Ok(definitions) => world.definitions.extend(definitions),
            Err(error) => {
                report.world_parse_failures += 1;
                report
                    .world_error_samples
                    .push(format!("{}: {error}", file_label(report, &path)));
            }
        }
    }

    for path in files_with_extension(directory, "ipl")? {
        let text = read_text(&path)?;

        match vc_world::parse_instances(&text) {
            Ok(instances) => world.instances.extend(instances),
            Err(error) => {
                report.world_parse_failures += 1;
                report
                    .world_error_samples
                    .push(format!("{}: {error}", file_label(report, &path)));
            }
        }
    }

    report.world_definitions = world.definitions.len() as u64;
    report.world_instances = world.instances.len() as u64;
    report.world_unresolved = world.unresolved_instances() as u64;

    for definition in &world.definitions {
        if archive
            .indices_of(&format!("{}.dff", definition.model))
            .is_empty()
        {
            report.world_missing_models += 1;
        }

        if archive
            .indices_of(&format!("{}.txd", definition.txd))
            .is_empty()
        {
            report.world_missing_txds += 1;
        }
    }

    Ok(())
}

/// Reads the weather and lighting table.
fn tally_timecycle(report: &mut Report, path: &Path) -> Result<()> {
    let text = read_text(path)?;

    let Ok(table) = vc_timecyc::Timecycle::parse(&text) else {
        report.timecyc_failures += 1;

        return Ok(());
    };

    report.timecyc_weathers = table.weathers.len() as u64;
    report.timecyc_hours = table.hours().count() as u64;

    if let Some((min, max)) = table.far_clip_range() {
        report.timecyc_far_clip_min = min;
        report.timecyc_far_clip_max = max;
    }

    Ok(())
}

/// Accumulates the cull zones an IPL file declares.
fn tally_cull_zones(report: &mut Report, text: &str) {
    let Ok(zones) = vc_ipl::parse_cull_zones(text) else {
        report.cull_failures += 1;

        return;
    };

    report.cull_zones += zones.len() as u64;
    report.cull_centre_mismatches += zones
        .iter()
        .filter(|zone| !zone.anchor_xy_is_box_centre())
        .count() as u64;
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
        self.print_timecycle();
    }

    fn print_archive(&self) {
        println!("install      {}", self.install.display());
        println!("img entries  {}", self.img_entries);
        println!("fingerprint  {}", self.fingerprint);
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
        println!("  decoded tris      {}", self.decoded_triangles);
        println!("  decoded verts     {}", self.decoded_vertices);
        println!("  bad indices       {}", self.decoded_index_errors);
        println!("  non-finite pos    {}", self.decoded_non_finite);
        println!("  decode failures   {}", self.decode_failures);
        println!("  material slots    {}", self.material_slots);
        println!("  named slots       {}", self.named_slots);
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
        println!("  DXT1 family       {}", self.bc1_textures);
        println!("  DXT3/5 family     {}", self.bc23_textures);
        println!("  paletted          {}", self.paletted_textures);
        println!("  unclassified      {}", self.unclassified_textures);
        println!("  mip chains exact  {}", self.chains_exact);
        println!("  with mip chains   {}", self.textures_with_mips);
        println!("  cooked            {}", self.cooked_textures);
        println!("  cooked BC1        {}", self.cooked_bc1);
        println!("  cooked BC3        {}", self.cooked_bc3);
        println!("  cooked RGBA8      {}", self.cooked_rgba8);
        println!("  cooked bytes      {}", self.cooked_bytes);
        println!("  source bytes      {}", self.source_level_bytes);
        println!("  cook failures     {}", self.cook_failures);
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
        println!("  cull boxes        {}", self.cull_zones);
        println!("  anchor xy off box {}", self.cull_centre_mismatches);
        println!("  cull failures     {}", self.cull_failures);
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

        println!("\nworld");
        println!("  definitions       {}", self.world_definitions);
        println!("  instances         {}", self.world_instances);
        println!("  unresolved        {}", self.world_unresolved);
        println!("  missing models    {}", self.world_missing_models);
        println!("  missing txds      {}", self.world_missing_txds);
        println!("  parse failures    {}", self.world_parse_failures);

        for sample in &self.world_error_samples {
            println!("    {sample}");
        }
    }

    fn print_timecycle(&self) {
        println!("\ntimecyc");
        println!("  weathers          {}", self.timecyc_weathers);
        println!("  hours             {}", self.timecyc_hours);
        println!(
            "  far clip m        [{:.0},{:.0}]",
            self.timecyc_far_clip_min, self.timecyc_far_clip_max
        );
        println!("  parse failures    {}", self.timecyc_failures);
    }

    #[expect(
        clippy::too_many_lines,
        reason = "a flat table of one assertion per measured total; splitting it would only relocate the data"
    )]
    fn expected_checks(&self) -> [(&'static str, u64, u64); 63] {
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
                "decoded triangles",
                self.decoded_triangles,
                expected::TRIANGLES,
            ),
            (
                "decoded vertices",
                self.decoded_vertices,
                expected::VERTICES,
            ),
            ("decoded bad indices", self.decoded_index_errors, 0),
            ("decoded non-finite", self.decoded_non_finite, 0),
            ("decode failures", u64::from(self.decode_failures), 0),
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
            ("txd bc1 family", self.bc1_textures, expected::TXD_BC1),
            ("txd bc2/bc3 family", self.bc23_textures, expected::TXD_BC23),
            (
                "txd paletted",
                self.paletted_textures,
                expected::TXD_PALETTED,
            ),
            (
                "txd unclassified",
                self.unclassified_textures,
                expected::TXD_UNCLASSIFIED,
            ),
            (
                "txd mip chains exact",
                self.chains_exact,
                expected::TXD_TEXTURES,
            ),
            (
                "txd with mips",
                self.textures_with_mips,
                expected::TXD_WITH_MIPS,
            ),
            (
                "cooked textures",
                self.cooked_textures,
                expected::TXD_TEXTURES,
            ),
            ("cooked bc1", self.cooked_bc1, expected::TXD_BC1),
            ("cooked bc3", self.cooked_bc3, expected::TXD_BC23),
            ("cooked rgba8", self.cooked_rgba8, expected::TXD_PALETTED),
            ("cooked bytes", self.cooked_bytes, expected::COOKED_BYTES),
            (
                "source level bytes",
                self.source_level_bytes,
                expected::SOURCE_LEVEL_BYTES,
            ),
            ("cook failures", u64::from(self.cook_failures), 0),
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
            ("cull boxes", self.cull_zones, expected::CULL_BOXES),
            (
                "cull anchor xy off centre",
                self.cull_centre_mismatches,
                expected::CULL_CENTRE_MISMATCHES,
            ),
            ("cull failures", u64::from(self.cull_failures), 0),
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
            (
                "world definitions",
                self.world_definitions,
                expected::WORLD_DEFINITIONS,
            ),
            (
                "world instances",
                self.world_instances,
                expected::WORLD_INSTANCES,
            ),
            ("world unresolved", self.world_unresolved, 0),
            ("world missing models", self.world_missing_models, 0),
            ("world missing txds", self.world_missing_txds, 0),
            (
                "world parse failures",
                u64::from(self.world_parse_failures),
                0,
            ),
            (
                "timecyc weathers",
                self.timecyc_weathers,
                expected::TIMECYC_WEATHERS,
            ),
            ("timecyc hours", self.timecyc_hours, expected::TIMECYC_HOURS),
            (
                "timecyc parse failures",
                u64::from(self.timecyc_failures),
                0,
            ),
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

        if self.fingerprint != expected::ARCHIVE_FINGERPRINT {
            mismatches.push(format!(
                "archive fingerprint: found {}, expected {} (a different install build must update this constant deliberately)",
                self.fingerprint,
                expected::ARCHIVE_FINGERPRINT
            ));
        }

        if mismatches.is_empty() {
            return Ok(());
        }

        let details = mismatches.join("\n  ");

        bail!("installation does not match PLAN.md section 4:\n  {details}")
    }
}
