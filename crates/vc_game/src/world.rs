//! Planning and baking the static world.
//!
//! The map is small enough to hold in memory whole: 8,400 exterior placements, two
//! million triangles. Streaming it was the right call while streaming was the only
//! option, but a resident map removes the pop-in, the per-frame bookkeeping and the
//! whole class of bugs where a cell boundary cuts a model in half.
//!
//! Placements are baked into one mesh per texture per chunk, so the runtime draws a
//! few hundred meshes per city block instead of one entity per material group.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bevy::prelude::*;
use vc_physics::{Placement as PhysicsPlacement, StaticWorld};
use vc_render::{Batcher, MaterialCache, Placement, TextureCache};
use vc_stream::{Colliders, CookedModel, Streamer};
use vc_world::{CELL_SIZE, CellKey, Instance, ObjectDefinition, World};

/// Cells per chunk side.
///
/// A chunk is the unit the frustum culls and the unit geometry merges within: 400
/// units is small enough to cull a street, large enough that the same texture is not
/// merged into dozens of separate meshes.
pub const CHUNK_CELLS: i32 = 2;

/// Physics bucket size, in game units.
pub const PHYSICS_BUCKET: f32 = 25.0;

/// A chunk of the map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChunkKey {
    /// Chunk index along the game's x axis.
    pub x: i32,
    /// Chunk index along the game's y axis.
    pub y: i32,
}

impl ChunkKey {
    /// The chunk a game-space position falls in.
    #[must_use]
    pub fn of(position: [f32; 3]) -> Self {
        let cell = CellKey::of(position, CELL_SIZE);

        Self {
            x: cell.x.div_euclid(CHUNK_CELLS),
            y: cell.y.div_euclid(CHUNK_CELLS),
        }
    }
}

/// What the runtime will draw, after filtering and fixing the data.
#[derive(Resource)]
pub struct WorldPlan {
    /// Definitions by id.
    pub definitions: HashMap<u32, ObjectDefinition>,
    /// Placements to draw, with rotations in the sense the game uses.
    pub instances: Vec<Instance>,
    /// Placement indices per chunk.
    pub chunks: BTreeMap<ChunkKey, Vec<u32>>,
    /// Placements dropped as level-of-detail duplicates.
    pub dropped_lod: usize,
    /// Placements dropped as interiors.
    pub dropped_interior: usize,
    /// Placements dropped because their definition is missing.
    pub dropped_unknown: usize,
}

impl WorldPlan {
    /// Filters a loaded map and fixes its rotations.
    #[must_use]
    pub fn new(world: World) -> Self {
        let definitions: HashMap<u32, ObjectDefinition> = world
            .definitions
            .into_iter()
            .map(|definition| (definition.id, definition))
            .collect();

        let lod: Vec<&Instance> = world
            .instances
            .iter()
            .filter(|instance| {
                definitions
                    .get(&instance.definition_id)
                    .is_some_and(|definition| is_lod(&definition.model))
            })
            .collect();
        let detail: Vec<&Instance> = world
            .instances
            .iter()
            .filter(|instance| {
                definitions
                    .get(&instance.definition_id)
                    .is_some_and(|definition| !is_lod(&definition.model))
            })
            .collect();

        let mut instances = Vec::with_capacity(world.instances.len());
        let (mut dropped_lod, mut dropped_interior, mut dropped_unknown) = (0, 0, 0);

        for instance in &world.instances {
            let Some(definition) = definitions.get(&instance.definition_id) else {
                dropped_unknown += 1;

                continue;
            };

            if is_lod(&definition.model) {
                // A shell with no detail counterpart is the only copy of that object,
                // so dropping it leaves a hole. The island shells are the exception:
                // they are coarse stand-ins for whole blocks that are drawn in detail.
                let orphan = !definition
                    .model
                    .to_ascii_lowercase()
                    .starts_with("islandlod")
                    && !detail
                        .iter()
                        .any(|detail| near(&detail.position, &instance.position));

                if orphan {
                    instances.push(corrected(instance));
                } else {
                    dropped_lod += 1;
                }

                continue;
            }

            // A detail placement that shares its position with a level-of-detail shell
            // is outdoors by construction: shells exist only for outside scenery. That
            // rescue matters because a block of the map carries a bogus interior id.
            let outdoor = lod
                .iter()
                .any(|shell| near(&shell.position, &instance.position));

            if instance.interior > 0 && !outdoor {
                dropped_interior += 1;

                continue;
            }

            instances.push(corrected(instance));
        }

        let mut chunks: BTreeMap<ChunkKey, Vec<u32>> = BTreeMap::new();
        for (index, instance) in instances.iter().enumerate() {
            chunks
                .entry(ChunkKey::of(instance.position))
                .or_default()
                .push(index as u32);
        }

        Self {
            definitions,
            instances,
            chunks,
            dropped_lod,
            dropped_interior,
            dropped_unknown,
        }
    }

    /// Where to start: the requested cell, or the one the map fills most.
    #[must_use]
    pub fn start(&self, requested: Option<CellKey>) -> [f32; 3] {
        if let Some(cell) = requested {
            return cell.centre(CELL_SIZE);
        }

        let mut counts: HashMap<CellKey, usize> = HashMap::new();
        for instance in &self.instances {
            *counts
                .entry(CellKey::of(instance.position, CELL_SIZE))
                .or_default() += 1;
        }

        counts
            .into_iter()
            .max_by_key(|(cell, count)| (*count, std::cmp::Reverse(*cell)))
            .map_or([0.0; 3], |(cell, _)| cell.centre(CELL_SIZE))
    }
}

/// Flips an instance into the rotation sense the game uses.
///
/// The shipped quaternion is the inverse of the rotation that was applied: lamp arms
/// point at the carriageway only when it is conjugated.
fn corrected(instance: &Instance) -> Instance {
    let mut instance = instance.clone();
    instance.rotation = [
        -instance.rotation[0],
        -instance.rotation[1],
        -instance.rotation[2],
        instance.rotation[3],
    ];

    instance
}

fn near(left: &[f32; 3], right: &[f32; 3]) -> bool {
    (left[0] - right[0]).abs() < 1.0
        && (left[1] - right[1]).abs() < 1.0
        && (left[2] - right[2]).abs() < 1.0
}

/// Whether a model is a level-of-detail shell.
///
/// Vice City marks them by name only: no IDE flag or IPL field separates them, and no
/// ordinary model contains the substring.
#[must_use]
pub fn is_lod(model: &str) -> bool {
    model.to_ascii_lowercase().contains("lod")
}

/// Spawns one baked mesh per part, with blend materials excluded from shadows.
pub fn spawn(commands: &mut Commands<'_, '_>, baked: BakedWorld) {
    for part in baked.parts {
        let mut entity = commands.spawn((
            Mesh3d(part.mesh),
            MeshMaterial3d(part.material),
            part.bounds,
            Transform::IDENTITY,
        ));
        if part.blend {
            entity.insert(bevy::light::NotShadowCaster);
        }
    }
}

/// Everything one bake produced, before it is spawned.
pub struct BakedWorld {
    /// One mesh per texture per chunk.
    pub parts: Vec<vc_render::MeshPart>,
    /// What the bake cost.
    pub stats: BakeStats,
}

/// Counters from one bake.
#[derive(Default, Clone, Copy)]
pub struct BakeStats {
    /// Chunks with geometry.
    pub chunks: usize,
    /// Meshes spawned.
    pub draws: usize,
    /// Triangles across them.
    pub triangles: usize,
    /// Vertices across them.
    pub vertices: usize,
    /// Textures uploaded.
    pub textures: usize,
    /// Models that failed to cook.
    pub failed: usize,
}

/// Bakes every chunk into merged meshes and registers collision.
#[expect(
    clippy::too_many_arguments,
    reason = "each argument is a distinct asset collection or cache"
)]
pub fn bake(
    plan: &WorldPlan,
    streamer: &mut Streamer,
    colliders: &Colliders,
    physics: &mut StaticWorld,
    textures: &mut TextureCache,
    materials: &mut MaterialCache,
    meshes: &mut Assets<Mesh>,
    standard: &mut Assets<StandardMaterial>,
    images: &mut Assets<Image>,
) -> BakedWorld {
    let mut stats = BakeStats::default();
    let mut parts = Vec::new();

    // Cook once per definition, then bake from the cache. The streamer already shares
    // models by name, but a definition lookup per placement is still wasted work.
    let mut models: HashMap<u32, Option<Arc<CookedModel>>> =
        HashMap::with_capacity(plan.definitions.len());
    for instance in &plan.instances {
        if models.contains_key(&instance.definition_id) {
            continue;
        }
        let cooked = plan
            .definitions
            .get(&instance.definition_id)
            .and_then(|definition| streamer.model(definition).ok());
        if cooked.is_none() {
            stats.failed += 1;
        }
        let _ = models.insert(instance.definition_id, cooked);
    }

    let unique = collect_textures(models.values().flatten());
    textures.upload_batch(&unique, images);
    stats.textures = textures.len();

    let mut collider_ids: HashMap<String, usize> = HashMap::new();

    for indices in plan.chunks.values() {
        let mut batcher = Batcher::default();

        for index in indices {
            let Some(instance) = plan.instances.get(*index as usize) else {
                continue;
            };
            let Some(Some(model)) = models.get(&instance.definition_id) else {
                continue;
            };
            let placement = Placement::new(instance.position, instance.rotation, instance.scale);
            batcher.push(model, &placement);

            if let Some(definition) = plan.definitions.get(&instance.definition_id) {
                register_collision(definition, instance, colliders, physics, &mut collider_ids);
            }
        }

        let baked = batcher.finish(meshes, standard, textures, materials);
        stats.chunks += 1;
        stats.draws += baked.parts.len();
        stats.triangles += baked.triangles;
        stats.vertices += baked.vertices;

        parts.extend(baked.parts);
    }

    BakedWorld { parts, stats }
}

fn collect_textures<'a>(
    models: impl Iterator<Item = &'a Arc<CookedModel>>,
) -> Vec<Arc<vc_cook::CookedTexture>> {
    let mut seen = HashSet::new();
    let mut textures = Vec::new();

    for model in models {
        for mesh in &model.meshes {
            for group in &mesh.groups {
                let Some(texture) = group.texture.as_ref() else {
                    continue;
                };
                if seen.insert(Arc::as_ptr(texture) as usize) {
                    textures.push(Arc::clone(texture));
                }
            }
        }
    }

    textures.sort_by_key(|texture| Arc::as_ptr(texture) as usize);

    textures
}

fn register_collision(
    definition: &ObjectDefinition,
    instance: &Instance,
    colliders: &Colliders,
    physics: &mut StaticWorld,
    ids: &mut HashMap<String, usize>,
) {
    let Some(collider) = colliders.get(&definition.model) else {
        return;
    };
    let index = if let Some(&known) = ids.get(&definition.model) {
        known
    } else {
        let added = physics.add_collider(collider.clone());
        let _ = ids.insert(definition.model.clone(), added);

        added
    };
    let (min, max) = collider.bounds();

    physics.place(
        index,
        PhysicsPlacement {
            position: instance.position,
            rotation: instance.rotation,
            scale: instance.scale,
        },
        (min, max),
    );
}

/// Reads every IDE and IPL file under a directory.
///
/// # Errors
/// Returns an error when the directory or one of its files cannot be read.
pub fn load(directory: &Path) -> Result<World, Box<dyn Error>> {
    let mut world = World::default();
    let mut paths = Vec::new();
    collect(directory, &mut paths)?;

    for path in &paths {
        let text = String::from_utf8_lossy(&fs::read(path)?).into_owned();

        match path.extension().and_then(|extension| extension.to_str()) {
            Some("ide") => match vc_world::parse_definitions(&text) {
                Ok(definitions) => world.definitions.extend(definitions),
                Err(error) => println!("skip {}: {error}", path.display()),
            },
            Some("ipl") => match vc_world::parse_instances(&text) {
                Ok(instances) => world.instances.extend(instances),
                Err(error) => println!("skip {}: {error}", path.display()),
            },
            _ => {}
        }
    }

    Ok(world)
}

fn collect(directory: &Path, out: &mut Vec<PathBuf>) -> Result<(), Box<dyn Error>> {
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();

        if path.is_dir() {
            collect(&path, out)?;
        } else if matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("ide" | "ipl")
        ) {
            out.push(path);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn instance(id: u32, model: &str, interior: i32, position: [f32; 3]) -> Instance {
        Instance {
            definition_id: id,
            model: model.to_owned(),
            interior,
            position,
            scale: [1.0; 3],
            rotation: [0.0, 0.0, 0.0, 1.0],
        }
    }

    fn world(instances: Vec<Instance>) -> World {
        World {
            definitions: vec![
                ObjectDefinition {
                    id: 1,
                    model: "road".to_owned(),
                    txd: "generic".to_owned(),
                    draw_distance: 200.0,
                    flags: 0,
                },
                ObjectDefinition {
                    id: 2,
                    model: "LODroad".to_owned(),
                    txd: "generic".to_owned(),
                    draw_distance: 800.0,
                    flags: 0,
                },
            ],
            instances,
        }
    }

    #[test]
    fn keeps_a_shell_that_has_no_detail_counterpart() {
        let plan = WorldPlan::new(world(vec![instance(2, "LODroad", 0, [0.0; 3])]));

        assert_eq!(plan.instances.len(), 1);
        assert_eq!(plan.dropped_lod, 0);
    }

    #[test]
    fn drops_a_shell_that_duplicates_detail_geometry() {
        let plan = WorldPlan::new(world(vec![
            instance(1, "road", 0, [0.0; 3]),
            instance(2, "LODroad", 0, [0.0; 3]),
        ]));

        assert_eq!(plan.instances.len(), 1);
        assert_eq!(plan.instances[0].definition_id, 1);
        assert_eq!(plan.dropped_lod, 1);
    }

    #[test]
    fn rescues_an_outdoor_placement_tagged_with_an_interior() {
        let plan = WorldPlan::new(world(vec![
            instance(1, "road", 13, [10.0, 20.0, 5.0]),
            instance(2, "LODroad", 0, [10.0, 20.0, 5.0]),
        ]));

        assert_eq!(plan.instances.len(), 1);
        assert_eq!(plan.instances[0].position, [10.0, 20.0, 5.0]);
    }

    #[test]
    fn drops_a_real_interior() {
        let plan = WorldPlan::new(world(vec![instance(1, "road", 4, [10.0, 20.0, 5.0])]));

        assert_eq!(plan.instances.len(), 0);
        assert_eq!(plan.dropped_interior, 1);
    }

    #[test]
    fn conjugates_the_stored_rotation() {
        let mut instance = instance(1, "road", 0, [0.0; 3]);
        instance.rotation = [0.0, 0.0, 0.707_106_77, 0.707_106_77];

        let plan = WorldPlan::new(world(vec![instance]));

        assert_eq!(plan.instances[0].rotation[2], -0.707_106_77);
        assert_eq!(plan.instances[0].rotation[3], 0.707_106_77);
    }

    #[test]
    fn buckets_adjacent_cells_into_one_chunk() {
        let left = ChunkKey::of([0.0, 0.0, 0.0]);
        let right = ChunkKey::of([CELL_SIZE * 1.5, 0.0, 0.0]);

        assert_eq!(left, right);
        assert_ne!(left, ChunkKey::of([CELL_SIZE * 2.5, 0.0, 0.0]));
    }
}
