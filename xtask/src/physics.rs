//! Builds collision for a region and reports what it finds there.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use vc_physics::{Placement, StaticWorld};
use vc_stream::Colliders;
use vc_transport::{GameRoot, ImageArchive};
use vc_world::{CELL_SIZE, CellGrid, World};

pub(crate) fn run(radius: i32) -> Result<()> {
    let root = GameRoot::from_env()?;
    let archive = ImageArchive::open(&root.image())?;
    let world = load_world(&root.path().join("data/maps"))?;
    report_files(&archive)?;

    let colliders = Colliders::load(&archive);

    println!("collision models: {}", colliders.len());

    let world = World {
        definitions: world.definitions,
        instances: world
            .instances
            .into_iter()
            .filter(|instance| instance.interior == 0)
            .collect(),
    };

    let grid = CellGrid::build(&world, CELL_SIZE);
    let start = grid.busiest().context("the map holds no placements")?;
    let mut definitions = HashMap::with_capacity(world.definitions.len());
    for definition in world.definitions {
        let _ = definitions.insert(definition.id, definition);
    }

    let mut physics = StaticWorld::new(25.0);
    let mut indices: HashMap<String, usize> = HashMap::new();
    let mut placed = 0u64;
    let mut missing = 0u64;

    for key in grid.within(start, radius) {
        for instance in grid.instances(key) {
            let Some(instance) = world.instances.get(*instance as usize) else {
                continue;
            };
            let Some(definition) = definitions.get(&instance.definition_id) else {
                continue;
            };
            let Some(collider) = colliders.get(&definition.model) else {
                if missing < 6 {
                    println!("  no collision for `{}`", definition.model);
                }
                missing += 1;

                continue;
            };

            let index = *indices
                .entry(definition.model.clone())
                .or_insert_with(|| physics.add_collider(collider.clone()));
            let (min, max) = collider.bounds();

            physics.place(
                index,
                Placement {
                    position: instance.position,
                    rotation: instance.rotation,
                    scale: instance.scale,
                },
                (min, max),
            );
            placed += 1;
        }
    }

    println!("instances placed {placed}, without collision {missing}");

    // Probe straight down over the instances that did get collision: if the world is
    // wired correctly each of these should find its own surface.
    let mut hits = 0;
    let mut probed = 0;
    let mut heights = Vec::new();

    for key in grid.within(start, radius) {
        for instance in grid.instances(key) {
            let Some(instance) = world.instances.get(*instance as usize) else {
                continue;
            };
            let Some(definition) = definitions.get(&instance.definition_id) else {
                continue;
            };

            if colliders.get(&definition.model).is_none() {
                continue;
            }

            probed += 1;
            let probe = [
                instance.position[0],
                instance.position[1] + 60.0,
                instance.position[2],
            ];

            if let Some(height) = physics.ground(probe, 120.0) {
                heights.push(height);
                hits += 1;
            }
        }
    }

    println!("ground probes {hits}/{probed} hit");

    if !heights.is_empty() {
        let low = heights.iter().copied().fold(f32::INFINITY, f32::min);
        let high = heights.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        println!("surface height range {low:.2}..{high:.2}");
    }

    Ok(())
}

/// Prints the collision files found and how the first one parses.
fn report_files(archive: &ImageArchive) -> Result<()> {
    let cols: Vec<&str> = archive
        .entries()
        .iter()
        .filter(|entry| entry.has_extension("col"))
        .map(vc_transport::ImgEntry::name)
        .collect();

    println!("col files: {}", cols.len());

    if let Some(name) = cols.first()
        && let Some(&index) = archive.indices_of(name).first()
    {
        match vc_col::Collision::parse(archive.data(index)?) {
            Ok(parsed) => println!("  {name}: {} records", parsed.records.len()),
            Err(error) => println!("  {name}: {error}"),
        }
    }

    Ok(())
}

fn load_world(directory: &Path) -> Result<World> {
    let mut world = World::default();
    let mut paths = Vec::new();
    collect(directory, &mut paths)?;

    for path in &paths {
        let bytes = std::fs::read(path)?;
        let text = String::from_utf8_lossy(&bytes);

        match path.extension().and_then(|found| found.to_str()) {
            Some("ide") => world
                .definitions
                .extend(vc_world::parse_definitions(&text)?),
            Some("ipl") => world.instances.extend(vc_world::parse_instances(&text)?),
            _ => {}
        }
    }

    Ok(world)
}

fn collect(directory: &Path, out: &mut Vec<std::path::PathBuf>) -> Result<()> {
    for entry in std::fs::read_dir(directory)? {
        let path = entry?.path();

        if path.is_dir() {
            collect(&path, out)?;
        } else if matches!(
            path.extension().and_then(|found| found.to_str()),
            Some("ide" | "ipl")
        ) {
            out.push(path);
        }
    }

    Ok(())
}
