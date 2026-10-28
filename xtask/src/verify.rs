//! Ranks placements by how far the rendered geometry sits from the collision model.
//!
//! A model's DFF and its COL are independent descriptions of the same object and both
//! are transformed by the same instance, so a wrong *instance* rotation cancels out.
//! What does not cancel is an error on the geometry side alone - a wrong frame
//! transform, a misplaced pivot - and that shows up as the rendered mass sitting away
//! from the collision mass. Ranking every model by that offset finds the broken ones
//! without anyone having to fly around looking for them.
// Diagnostic maths, kept in its textbook form.
#![expect(
    clippy::suboptimal_flops,
    clippy::similar_names,
    reason = "vector maths with conventional single-letter names"
)]

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use anyhow::{Context, Result};
use vc_stream::{Colliders, Streamer};
use vc_transport::{GameRoot, ImageArchive};
use vc_world::{CellKey, ObjectDefinition, World};

pub(crate) fn run(cell: Option<(i32, i32)>) -> Result<()> {
    let root = GameRoot::from_env()?;
    let archive = ImageArchive::open(&root.image())?;
    let world = load_world(&root.path().join("data/maps"))?;

    let mut definitions: HashMap<u32, ObjectDefinition> =
        HashMap::with_capacity(world.definitions.len());
    for definition in &world.definitions {
        let _ = definitions.insert(definition.id, definition.clone());
    }

    let colliders = Colliders::load(&archive);
    let mut streamer = Streamer::open(archive);

    let mut checked = 0u32;
    let mut offsets: BTreeMap<String, (u32, f32, f32)> = BTreeMap::new();
    let mut untextured: BTreeMap<String, u32> = BTreeMap::new();

    for instance in &world.instances {
        if instance.interior > 0 || !in_cell(instance.position, cell) {
            continue;
        }
        let Some(definition) = definitions.get(&instance.definition_id) else {
            continue;
        };

        let Ok(model) = streamer.model(definition) else {
            continue;
        };

        let missing: usize = model
            .meshes
            .iter()
            .flat_map(|mesh| mesh.groups.iter())
            .filter(|group| group.texture.is_none())
            .count();

        if missing > 0 {
            *untextured.entry(definition.model.clone()).or_insert(0) += missing as u32;
        }

        let Some(collider) = colliders.get(&definition.model) else {
            continue;
        };

        checked += 1;

        // Offset relative to the model's own size: a big irregular model legitimately
        // disagrees with a box-shaped collision proxy, a street lamp does not.
        let offset = distance(
            rendered_centroid(&model, instance),
            collider_centroid(collider, instance),
        );
        let size = rendered_size(&model).max(0.5);

        let entry = offsets
            .entry(definition.model.clone())
            .or_insert((0, 0.0, 0.0));
        entry.0 += 1;
        entry.1 += offset / size;
        entry.2 = entry.2.max(offset / size);
    }

    println!("{checked} instances with both geometry and collision");

    let mut ranked: Vec<(&String, &(u32, f32, f32))> = offsets.iter().collect();
    ranked.sort_by(|a, b| (b.1.2).total_cmp(&a.1.2));

    println!("\nworst rendered-vs-collision offsets, as a fraction of model size:");
    for (model, (count, total, worst)) in ranked.iter().take(24) {
        println!(
            "  {model:<26} x{count:<5} mean {:5.2}  worst {:5.2}",
            total / *count as f32,
            worst
        );
    }

    ab_report(&ranked, &definitions, &colliders, &world, &streamer);

    println!("\nmodels drawing with no texture (they render white):");
    for (model, count) in &untextured {
        println!("  {model:<26} {count} groups");
    }

    Ok(())
}

/// Longest edge of the rendered geometry's bounding box.
fn rendered_size(model: &vc_stream::CookedModel) -> f32 {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];

    for mesh in &model.meshes {
        for vertex in &mesh.vertices {
            for axis in 0..3 {
                min[axis] = min[axis].min(vertex.position[axis]);
                max[axis] = max[axis].max(vertex.position[axis]);
            }
        }
    }

    if !min[0].is_finite() {
        return 0.0;
    }

    (0..3)
        .map(|axis| max[axis] - min[axis])
        .fold(0.0f32, f32::max)
}

/// Centroid of a clump's geometry, optionally through the frame hierarchy.
fn centroid_of(
    clump: &vc_dff::DecodedClump,
    transforms: &[([[f32; 3]; 3], [f32; 3])],
    instance: &vc_world::Instance,
    use_frames: bool,
) -> [f32; 3] {
    let mut sum = [0.0f32; 3];
    let mut count = 0u32;

    for (index, geometry) in clump.geometries.iter().enumerate() {
        let (rotation, position) = if use_frames {
            transforms
                .get(index)
                .copied()
                .unwrap_or((vc_dff::identity(), [0.0; 3]))
        } else {
            (vc_dff::identity(), [0.0; 3])
        };

        for vertex in &geometry.decoded.vertices {
            let local = vc_dff::add(vc_dff::rotate(rotation, vertex.position), position);
            let placed = transform(local, instance);

            for axis in 0..3 {
                sum[axis] += placed[axis];
            }
            count += 1;
        }
    }

    if count == 0 {
        return instance.position;
    }

    sum.map(|value| value / count as f32)
}

fn clump_size(clump: &vc_dff::DecodedClump) -> f32 {
    let mut min = [f32::INFINITY; 3];
    let mut max = [f32::NEG_INFINITY; 3];

    for geometry in &clump.geometries {
        for vertex in &geometry.decoded.vertices {
            for axis in 0..3 {
                min[axis] = min[axis].min(vertex.position[axis]);
                max[axis] = max[axis].max(vertex.position[axis]);
            }
        }
    }

    if !min[0].is_finite() {
        return 0.0;
    }

    (0..3)
        .map(|axis| max[axis] - min[axis])
        .fold(0.0f32, f32::max)
}

fn in_cell(position: [f32; 3], cell: Option<(i32, i32)>) -> bool {
    cell.is_none_or(|(x, y)| CellKey::of(position, 200.0) == CellKey { x, y })
}

/// Centre of mass of the geometry as the renderer places it.
fn rendered_centroid(model: &vc_stream::CookedModel, instance: &vc_world::Instance) -> [f32; 3] {
    let mut sum = [0.0f32; 3];
    let mut count = 0u32;

    for mesh in &model.meshes {
        for vertex in &mesh.vertices {
            let placed = transform(vertex.position, instance);

            for axis in 0..3 {
                sum[axis] += placed[axis];
            }
            count += 1;
        }
    }

    if count == 0 {
        return instance.position;
    }

    sum.map(|value| value / count as f32)
}

fn collider_centroid(collider: &vc_physics::Collider, instance: &vc_world::Instance) -> [f32; 3] {
    let mut sum = [0.0f32; 3];
    let mut count = 0u32;

    for vertex in &collider.vertices {
        let placed = transform(*vertex, instance);

        for axis in 0..3 {
            sum[axis] += placed[axis];
        }
        count += 1;
    }

    if count == 0 {
        return instance.position;
    }

    sum.map(|value| value / count as f32)
}

fn transform(point: [f32; 3], instance: &vc_world::Instance) -> [f32; 3] {
    let scaled = [
        point[0] * instance.scale[0],
        point[1] * instance.scale[1],
        point[2] * instance.scale[2],
    ];
    let [x, y, z, w] = instance.rotation;
    let axis = [x, y, z];
    let uv = cross(axis, scaled);
    let uuv = cross(axis, uv);

    [
        instance.position[0] + scaled[0] + 2.0 * (uuv[0] + uv[0] * w),
        instance.position[1] + scaled[1] + 2.0 * (uuv[1] + uv[1] * w),
        instance.position[2] + scaled[2] + 2.0 * (uuv[2] + uv[2] * w),
    ]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let dx = a[0] - b[0];
    let dy = a[1] - b[1];
    let dz = a[2] - b[2];

    (dx * dx + dy * dy + dz * dz).sqrt()
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

/// Prints, for the worst offenders, whether applying the frame hierarchy helps.
fn ab_report(
    ranked: &[(&String, &(u32, f32, f32))],
    definitions: &HashMap<u32, ObjectDefinition>,
    colliders: &Colliders,
    world: &World,
    streamer: &Streamer,
) {
    println!("\nframe-transform A/B on the worst offenders (offset / model size):");
    println!("  {:<26} {:>8} {:>8}", "model", "frames", "identity");

    for (model, _) in ranked.iter().take(24) {
        let Some(definition) = definitions.values().find(|entry| entry.model == **model) else {
            continue;
        };
        let name = format!("{}.dff", definition.model);
        let Some(&index) = streamer.archive().indices_of(&name).first() else {
            continue;
        };
        let Ok(bytes) = streamer.archive().data(index) else {
            continue;
        };
        let Ok(clump) = vc_dff::decode_clump_geometry(bytes) else {
            continue;
        };
        let transforms = clump.geometry_transforms();

        let Some(collider) = colliders.get(&definition.model) else {
            continue;
        };
        let Some(instance) = world
            .instances
            .iter()
            .find(|instance| instance.definition_id == definition.id)
        else {
            continue;
        };

        let with = centroid_of(&clump, &transforms, instance, true);
        let without = centroid_of(&clump, &transforms, instance, false);
        let collided = collider_centroid(collider, instance);
        let size = clump_size(&clump).max(0.5);

        println!(
            "  {:<26} {:8.2} {:8.2}",
            model,
            distance(with, collided) / size,
            distance(without, collided) / size
        );
    }
}
