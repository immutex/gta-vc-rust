//! Cross-checks placements against the map's own collision data.
//!
//! A model's DFF and its COL are independent descriptions of the same object, so their
//! world-space extents must agree. Where they do not, either the geometry was placed
//! wrongly or the collision was, and either way the instance is suspect. The same pass
//! reports models whose textures failed to resolve, because those draw with the
//! fallback and read as white.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use anyhow::{Context, Result};
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

    let colliders = Colliders::load(&archive);
    let mut streamer = Streamer::open(archive);
    let key = CellKey { x, y };

    let mut checked = 0u32;
    let mut disagreeing: BTreeMap<String, (u32, [f32; 3], [f32; 3])> = BTreeMap::new();
    let mut untextured: BTreeMap<String, u32> = BTreeMap::new();

    for instance in &world.instances {
        if instance.interior > 0 || CellKey::of(instance.position, 200.0) != key {
            continue;
        }
        let Some(definition) = definitions.get(&instance.definition_id) else {
            continue;
        };

        let Ok(model) = streamer.model(definition) else {
            continue;
        };

        let mut missing = 0;
        for mesh in &model.meshes {
            for group in &mesh.groups {
                if group.texture.is_none() {
                    missing += 1;
                }
            }
        }
        if missing > 0 {
            let entry = untextured.entry(definition.model.clone()).or_insert(0);
            *entry += missing;
        }

        let Some(collider) = colliders.get(&definition.model) else {
            continue;
        };

        checked += 1;

        let rendered = rendered_extent(&model);
        let (min, max) = collider.bounds();
        let collision_extent = (
            [max[0] - min[0], max[1] - min[1], max[2] - min[2]],
            [min[0], min[1], min[2]],
        );

        // Compare sizes rather than positions: the two are authored around their own
        // origins, but a rotated or scaled placement changes one extent relative to
        // the other, and unequal sizes are the signal.
        let size = [
            rendered.0[0] - rendered.1[0],
            rendered.0[1] - rendered.1[1],
            rendered.0[2] - rendered.1[2],
        ];
        let ratio = [
            ratio_of(size[0], collision_extent.0[0]),
            ratio_of(size[1], collision_extent.0[1]),
            ratio_of(size[2], collision_extent.0[2]),
        ];

        if ratio.iter().any(|value| *value > 1.5 || *value < 0.667) {
            let _ = disagreeing.entry(definition.model.clone()).or_insert((
                0,
                size,
                collision_extent.0,
            ));
            if let Some(entry) = disagreeing.get_mut(&definition.model) {
                entry.0 += 1;
            }
        }
    }

    println!("cell ({x}, {y}): {checked} instances with both geometry and collision");

    println!("\nextent disagreements (rendered vs collision, size ratio over 1.5x):");
    for (model, (count, rendered, collided)) in &disagreeing {
        println!(
            "  {model:<24} x{count:<3} rendered {:.1}x{:.1}x{:.1}  collision {:.1}x{:.1}x{:.1}",
            rendered[0], rendered[1], rendered[2], collided[0], collided[1], collided[2]
        );
    }

    println!("\nmodels drawing with no texture (they render white):");
    for (model, count) in &untextured {
        println!("  {model:<24} {count} groups");
    }

    Ok(())
}

fn rendered_extent(model: &vc_stream::CookedModel) -> ([f32; 3], [f32; 3]) {
    let mut max = [f32::NEG_INFINITY; 3];
    let mut min = [f32::INFINITY; 3];

    for mesh in &model.meshes {
        for vertex in &mesh.vertices {
            for axis in 0..3 {
                max[axis] = max[axis].max(vertex.position[axis]);
                min[axis] = min[axis].min(vertex.position[axis]);
            }
        }
    }

    if !max[0].is_finite() {
        return ([0.0; 3], [0.0; 3]);
    }

    (max, min)
}

fn ratio_of(a: f32, b: f32) -> f32 {
    if b.abs() < 0.05 {
        return 1.0;
    }

    (a / b).abs().max(1.0 / (a / b).abs().max(f32::EPSILON))
}

fn load_world(directory: &Path) -> Result<World> {
    let mut world = World::default();
    let mut paths = Vec::new();
    collect(directory, &mut paths)?;

    for path in &paths {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
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
