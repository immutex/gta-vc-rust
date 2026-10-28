//! Cooks a region around the busiest cell and reports what resolved.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use vc_stream::Streamer;
use vc_transport::{GameRoot, ImageArchive};
use vc_world::{CELL_SIZE, CellGrid, World};

pub(crate) fn run(radius: i32) -> Result<()> {
    let root = GameRoot::from_env()?;
    let archive = ImageArchive::open(&root.image())?;
    let world = load_world(&root.path().join("data/maps"))?;

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

    println!(
        "world: {} definitions, {} placements, {} cells",
        definitions.len(),
        world.instances.len(),
        grid.cell_count()
    );
    println!(
        "start cell ({}, {}) holds {} placements; loading radius {radius}",
        start.x,
        start.y,
        grid.instances(start).len()
    );

    let mut streamer = Streamer::open(archive);
    let mut placed = 0u64;
    let mut triangles = 0u64;
    let mut groups = 0u64;
    let mut textured = 0u64;
    let mut untextured = 0u64;
    let mut failures = 0u64;

    for key in grid.within(start, radius) {
        for index in grid.instances(key) {
            let Some(instance) = world.instances.get(*index as usize) else {
                continue;
            };
            let Some(definition) = definitions.get(&instance.definition_id) else {
                failures += 1;

                continue;
            };

            placed += 1;

            match streamer.model(definition) {
                Ok(model) => {
                    triangles += model.triangle_count() as u64;
                    for mesh in &model.meshes {
                        for group in &mesh.groups {
                            groups += 1;

                            if group.texture.is_some() {
                                textured += 1;
                            } else {
                                untextured += 1;
                            }
                        }
                    }
                }
                Err(error) => {
                    if failures < 5 {
                        println!("  failed {}: {error}", definition.model);
                    }
                    failures += 1;
                }
            }
        }
    }

    let stats = streamer.stats();
    println!("placements visited {placed}");
    println!("  models cooked   {}", stats.models);
    println!("  dictionaries    {}", stats.dictionaries);
    println!("  textures        {}", stats.textures);
    println!("  triangles       {triangles}");
    println!("  draw groups     {groups}");
    println!("  textured        {textured}");
    println!("  untextured      {untextured}");
    println!("  failures        {failures}");
    println!("  missing names   {:?}", stats.missing);
    println!("  dictionary keys {:?}", stats.keys);

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
