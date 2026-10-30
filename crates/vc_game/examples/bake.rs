//! Bakes the whole map without a window and reports what the renderer would draw.
//!
//! Run with `cargo run --release -p vc_game --example bake`. It exercises the same
//! cooking, texture upload and merging path the game uses, so it is the check that
//! catches a broken bake without needing a GPU.

use std::error::Error;
use std::time::Instant;

use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use vc_game::world::{self, WorldPlan, bake};
use vc_render::{MaterialCache, TextureCache};
use vc_stream::{Colliders, Streamer};
use vc_transport::{GameRoot, ImageArchive};

fn main() -> Result<(), Box<dyn Error>> {
    let root = GameRoot::from_env()?;
    let archive = ImageArchive::open(&root.image())?;
    let mut streamer = Streamer::open(archive);
    let started = Instant::now();
    let indexed = streamer.warm_textures();
    println!(
        "textures: {indexed} indexed in {:.1}s",
        started.elapsed().as_secs_f32()
    );

    let (world, effects) = world::load(&root.path().join("data/maps"))?;
    let plan = WorldPlan::new(&world, &effects, vc_game::world::Sense::default());
    println!(
        "world: {} placements, {} LOD, {} interior, {} unknown dropped",
        plan.instances.len(),
        plan.dropped_lod,
        plan.dropped_interior,
        plan.dropped_unknown
    );

    let colliders = Colliders::load(streamer.archive());
    let mut physics = vc_physics::StaticWorld::new(world::PHYSICS_BUCKET);
    let mut textures = TextureCache::default();
    let mut materials = MaterialCache::default();
    let mut meshes = Assets::<Mesh>::default();
    let mut standard = Assets::<StandardMaterial>::default();
    let mut images = Assets::<Image>::default();

    let started = Instant::now();
    let baked = bake(
        &plan,
        &mut streamer,
        &colliders,
        &mut physics,
        &mut textures,
        &mut materials,
        &mut meshes,
        &mut standard,
        &mut images,
    );
    let elapsed = started.elapsed().as_secs_f32();

    println!(
        "bake: {} chunks, {} draws, {} triangles, {} vertices, {} materials, {} textures in {elapsed:.1}s",
        baked.stats.chunks,
        baked.stats.draws,
        baked.stats.triangles,
        baked.stats.vertices,
        materials.len(),
        textures.len(),
    );
    println!(
        "assets: {} meshes, {} materials, {} images",
        meshes.len(),
        standard.len(),
        images.len(),
    );
    println!("models failed: {}", baked.stats.failed);
    println!(
        "tinted groups: {} of {}, samples {:?}",
        baked.stats.tinted, baked.stats.groups, baked.stats.tints
    );
    println!(
        "material groups: {} ({} name no texture, {:.2}%)",
        baked.stats.groups,
        baked.stats.untextured,
        100.0 * baked.stats.untextured as f32 / baked.stats.groups.max(1) as f32,
    );
    let stats = streamer.stats();
    println!(
        "streamer groups: {} untextured, {} named a texture nothing holds",
        stats.untextured_groups, stats.unresolved_groups
    );
    println!("authored lights: {}", plan.lights.len());

    let mut zero_normals = 0usize;
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for (_, mesh) in meshes.iter() {
        if let Some(VertexAttributeValues::Float32x3(normals)) =
            mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
        {
            zero_normals += normals
                .iter()
                .filter(|normal| Vec3::from_array(**normal).length_squared() < 1e-6)
                .count();
        }
        if let Some(VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        {
            for position in positions {
                let point = Vec3::from_array(*position);
                min = min.min(point);
                max = max.max(point);
            }
        }
    }
    println!("zero normals: {zero_normals}");
    println!("bounds: {min} .. {max}");

    Ok(())
}
