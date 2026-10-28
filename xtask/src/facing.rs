//! Decides the instance rotation convention from the data itself.
//!
//! Street lights are placed to face the road, and the road graph is independent of
//! the instance transform. So: take each lamp's arm direction in model space, rotate
//! it by the instance quaternion as stored and by its conjugate, and measure which
//! one points at the road. The convention that wins is the one the game used - no
//! eyeballing, no hand-picked locations.
// Diagnostic maths, kept in its textbook form.
#![expect(
    clippy::suboptimal_flops,
    clippy::many_single_char_names,
    reason = "vector maths with conventional single-letter names"
)]

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use vc_paths::Paths;

use vc_transport::GameRoot;
use vc_world::{ObjectDefinition, World};

pub(crate) fn run() -> Result<()> {
    let root = GameRoot::from_env()?;
    let (world, world_effects) = load_world(&root.path().join("data/maps"))?;
    let paths = load_paths(&root.path().join("data/maps/paths.ipl"))?;

    let nodes: Vec<[f32; 3]> = paths
        .blocks
        .iter()
        .flat_map(|block| block.points.iter())
        .filter(|point| point.is_in_use())
        .map(vc_paths::PathPoint::position)
        .collect();

    println!("road nodes: {}", nodes.len());

    let mut definitions: HashMap<u32, ObjectDefinition> =
        HashMap::with_capacity(world.definitions.len());
    for definition in world.definitions {
        let _ = definitions.insert(definition.id, definition);
    }

    // Lamp heads hang over the road, and the 2dfx entry says where the head is in the
    // model's own space. So: place the head under both rotation conventions and see
    // which one lands closer to the road graph. Authored data on both sides.
    let mut effects: HashMap<u32, Vec<[f32; 3]>> = HashMap::new();
    for effect in &world_effects {
        effects
            .entry(effect.definition_id)
            .or_default()
            .push(effect.offset);
    }

    let mut stored = (0.0f64, 0u32, 0u32);
    let mut conjugated = (0.0f64, 0u32, 0u32);

    for instance in &world.instances {
        let Some(offsets) = effects.get(&instance.definition_id) else {
            continue;
        };
        let Some((node, distance)) = nearest_node(&nodes, instance.position) else {
            continue;
        };
        if distance > 40.0 {
            continue;
        }

        // Only anchors whose arm is off-axis can tell the conventions apart, and only
        // rotated instances apply the rotation at all.
        let rotated = instance.rotation[3].abs() < 0.999;
        for offset in offsets {
            if !rotated || offset[0].hypot(offset[1]) < 0.5 {
                continue;
            }

            let as_stored = place(*offset, instance.rotation, instance.position);
            let flipped = place(
                *offset,
                [
                    -instance.rotation[0],
                    -instance.rotation[1],
                    -instance.rotation[2],
                    instance.rotation[3],
                ],
                instance.position,
            );

            let to_stored = horizontal_distance(as_stored, node);
            let to_flipped = horizontal_distance(flipped, node);

            stored.0 += f64::from(to_stored);
            stored.1 += 1;
            if to_stored < to_flipped {
                stored.2 += 1;
            }

            conjugated.0 += f64::from(to_flipped);
            conjugated.1 += 1;
        }
    }

    println!("effect anchors measured: {} across the map", stored.1);

    if stored.1 > 0 {
        println!(
            "  stored quaternion: mean {:.2} units from the road",
            stored.0 / f64::from(stored.1)
        );
        println!(
            "  conjugated:        mean {:.2} units from the road",
            conjugated.0 / f64::from(conjugated.1)
        );
        println!(
            "  stored wins {:.1}% of anchors",
            100.0 * f64::from(stored.2) / f64::from(stored.1)
        );
        println!(
            "  mean advantage of stored: {:.2} units (positive means stored is closer)",
            (conjugated.0 - stored.0) / f64::from(stored.1)
        );
    }

    Ok(())
}

fn nearest_node(nodes: &[[f32; 3]], position: [f32; 3]) -> Option<([f32; 3], f32)> {
    let mut best: Option<([f32; 3], f32)> = None;

    for node in nodes {
        let dx = node[0] - position[0];
        let dy = node[1] - position[1];
        let distance = dx.hypot(dy);

        if best.is_none_or(|(_, found)| distance < found) {
            best = Some((*node, distance));
        }
    }

    best
}

fn rotate(point: [f32; 3], q: [f32; 4]) -> [f32; 3] {
    let [x, y, z, w] = q;
    let u = [x, y, z];
    let uv = cross(u, point);
    let uuv = cross(u, uv);

    [
        point[0] + 2.0 * (uuv[0] + uv[0] * w),
        point[1] + 2.0 * (uuv[1] + uv[1] * w),
        point[2] + 2.0 * (uuv[2] + uv[2] * w),
    ]
}

const fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn load_paths(path: &Path) -> Result<Paths> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let text = String::from_utf8_lossy(&bytes);

    Ok(Paths::parse(&text)?)
}

fn place(offset: [f32; 3], rotation: [f32; 4], position: [f32; 3]) -> [f32; 3] {
    let rotated = rotate(offset, rotation);

    [
        rotated[0] + position[0],
        rotated[1] + position[1],
        rotated[2] + position[2],
    ]
}

fn horizontal_distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

fn load_world(directory: &Path) -> Result<(World, Vec<vc_world::Effect>)> {
    let mut world = World::default();
    let mut effects = Vec::new();
    let mut paths = Vec::new();
    collect(directory, &mut paths)?;

    for path in &paths {
        let bytes = std::fs::read(path)?;
        let text = String::from_utf8_lossy(&bytes);

        match path.extension().and_then(|found| found.to_str()) {
            Some("ide") => {
                world
                    .definitions
                    .extend(vc_world::parse_definitions(&text)?);
                effects.extend(vc_world::parse_effects(&text)?);
            }
            Some("ipl") => world.instances.extend(vc_world::parse_instances(&text)?),
            _ => {}
        }
    }

    world.instances.retain(|instance| instance.interior <= 0);
    Ok((world, effects))
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
