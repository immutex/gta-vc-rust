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

use vc_transport::{GameRoot, ImageArchive};
use vc_world::{ObjectDefinition, World};

pub(crate) fn run() -> Result<()> {
    let root = GameRoot::from_env()?;
    let archive = ImageArchive::open(&root.image())?;
    let world = load_world(&root.path().join("data/maps"))?;
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

    let mut stored = (0.0f64, 0u32);
    let mut conjugated = (0.0f64, 0u32);
    let mut models_seen = 0u32;

    for instance in &world.instances {
        let Some(definition) = definitions.get(&instance.definition_id) else {
            continue;
        };
        if !definition.model.to_ascii_lowercase().contains("lamp") {
            continue;
        }

        let Some(arm) = arm_direction(&archive, &definition.model)? else {
            continue;
        };
        models_seen += 1;

        let Some((node, distance)) = nearest_node(&nodes, instance.position) else {
            continue;
        };
        if distance > 30.0 {
            continue;
        }

        let to_road = normalize([
            node[0] - instance.position[0],
            node[1] - instance.position[1],
            0.0,
        ]);
        let as_stored = normalize(horizontal(rotate(arm, instance.rotation)));
        let flipped = normalize(horizontal(rotate(
            arm,
            [
                -instance.rotation[0],
                -instance.rotation[1],
                -instance.rotation[2],
                instance.rotation[3],
            ],
        )));

        stored.0 += f64::from(dot(as_stored, to_road));
        stored.1 += 1;
        conjugated.0 += f64::from(dot(flipped, to_road));
        conjugated.1 += 1;
    }

    println!(
        "lamp instances measured: {} ({} model kinds)",
        stored.1, models_seen
    );

    if stored.1 > 0 {
        println!(
            "  stored quaternion: mean dot with road direction {:.3}",
            stored.0 / f64::from(stored.1)
        );
        println!(
            "  conjugated:        mean dot with road direction {:.3}",
            conjugated.0 / f64::from(conjugated.1)
        );
        println!("  (1.0 means the arm always points at the road, 0.0 is chance)");
    }

    Ok(())
}

/// Horizontal direction the model's top leans relative to its base, in model space.
fn arm_direction(archive: &ImageArchive, model: &str) -> Result<Option<[f32; 3]>> {
    let name = format!("{model}.dff");
    let Some(&index) = archive.indices_of(&name).first() else {
        return Ok(None);
    };
    let bytes = archive.data(index)?;
    let Ok(clump) = vc_dff::decode_clump_geometry(bytes) else {
        return Ok(None);
    };

    let mut lowest = f32::INFINITY;
    let mut highest = f32::NEG_INFINITY;
    for geometry in &clump.geometries {
        for vertex in &geometry.decoded.vertices {
            lowest = lowest.min(vertex.position[2]);
            highest = highest.max(vertex.position[2]);
        }
    }

    if !lowest.is_finite() || highest - lowest < 0.5 {
        return Ok(None);
    }

    let span = highest - lowest;
    let mut base = [0.0f32; 3];
    let mut base_count = 0u32;
    let mut top = [0.0f32; 3];
    let mut top_count = 0u32;

    for geometry in &clump.geometries {
        for vertex in &geometry.decoded.vertices {
            let z = vertex.position[2];

            if z < lowest + span * 0.15 {
                for (axis, value) in vertex.position.iter().enumerate() {
                    base[axis] += value;
                }
                base_count += 1;
            } else if z > highest - span * 0.15 {
                for (axis, value) in vertex.position.iter().enumerate() {
                    top[axis] += value;
                }
                top_count += 1;
            }
        }
    }

    if base_count == 0 || top_count == 0 {
        return Ok(None);
    }

    let base = base.map(|value| value / base_count as f32);
    let top = top.map(|value| value / top_count as f32);
    let arm = normalize(horizontal([top[0] - base[0], top[1] - base[1], 0.0]));

    if arm.iter().all(|value| value.abs() < 1e-6) {
        return Ok(None);
    }

    Ok(Some(arm))
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

const fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

const fn horizontal(v: [f32; 3]) -> [f32; 3] {
    [v[0], v[1], 0.0]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = dot(v, v).sqrt();

    if length < 1e-6 {
        return [0.0; 3];
    }

    [v[0] / length, v[1] / length, v[2] / length]
}

fn load_paths(path: &Path) -> Result<Paths> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let text = String::from_utf8_lossy(&bytes);

    Ok(Paths::parse(&text)?)
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
