//! Lists what a cell places, with texture resolution and alpha status per model.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use anyhow::Result;
use vc_stream::{Colliders, Streamer};
use vc_transport::{GameRoot, ImageArchive};
use vc_world::{CellKey, ObjectDefinition, World};

pub(crate) fn run(x: i32, y: i32) -> Result<()> {
    let root = GameRoot::from_env()?;
    let archive = ImageArchive::open(&root.image())?;
    let world = load_world(&root.path().join("data/maps"))?;

    let mut definitions: HashMap<u32, ObjectDefinition> =
        HashMap::with_capacity(world.definitions.len());
    for definition in world.definitions {
        let _ = definitions.insert(definition.id, definition);
    }

    let key = CellKey { x, y };
    let mut rows: BTreeMap<String, (u32, usize, usize, usize, bool)> = BTreeMap::new();
    let colliders = Colliders::load(&archive);
    let mut streamer = Streamer::open(archive);

    for instance in &world.instances {
        if instance.interior > 0 || CellKey::of(instance.position, 200.0) != key {
            continue;
        }
        let Some(definition) = definitions.get(&instance.definition_id) else {
            continue;
        };
        if definition.model.to_ascii_lowercase().contains("lod") {
            continue;
        }

        let has_collision = colliders.get(&definition.model).is_some();
        let entry = rows
            .entry(definition.model.clone())
            .or_insert_with(|| (0, 0, 0, 0, has_collision));
        entry.0 += 1;

        if let Ok(model) = streamer.model(definition) {
            entry.2 += model.triangle_count();

            let mut cutout = 0;
            for mesh in &model.meshes {
                for group in &mesh.groups {
                    entry.3 += 1;
                    if group
                        .texture
                        .as_ref()
                        .is_some_and(|texture| texture.has_alpha)
                    {
                        cutout += 1;
                    }
                }
            }
            entry.1 += cutout;
        }
    }

    println!("cell ({x}, {y}): {} model kinds", rows.len());
    println!("placements  tex-groups  cutout-groups  tris  collider  model");

    for (model, (placed, cutout, triangles, groups, collides)) in rows {
        if cutout > 0 || !collides {
            println!("{placed:10}  {groups:10}  {cutout:13}  {triangles:6}  {collides:8}  {model}");
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

    world.instances.retain(|instance| instance.interior <= 0);
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
