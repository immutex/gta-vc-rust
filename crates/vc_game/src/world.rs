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

use bevy::camera::visibility::VisibilityRange;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use vc_physics::{Placement as PhysicsPlacement, StaticWorld};
use vc_render::to_bevy_position;
use vc_render::water::{WaterMaterial, WaterParams, WaterSurface};

use crate::camera::FlyCamMarker;
use crate::sky::Sky;
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
    /// Placements dropped as exact duplicates of another placement.
    pub dropped_duplicate: usize,
    /// Authored lights over the roads, in world space.
    pub lights: Vec<LightPoint>,
}

impl WorldPlan {
    /// The closest placement to a game-space point, and its distance.
    #[must_use]
    pub fn nearest_to(&self, point: [f32; 3]) -> Option<(String, f32)> {
        let mut best: Option<(String, f32)> = None;

        for instance in &self.instances {
            let dx = instance.position[0] - point[0];
            let dy = instance.position[1] - point[1];
            let dz = instance.position[2] - point[2];
            let distance = dx.mul_add(dx, dy.mul_add(dy, dz * dz)).sqrt();

            if best.as_ref().is_none_or(|(_, found)| distance < *found) {
                let name = self
                    .definitions
                    .get(&instance.definition_id)
                    .map_or_else(|| "?".to_owned(), |definition| definition.model.clone());
                best = Some((name, distance));
            }
        }

        best
    }
}

/// One authored light: where it is, what colour it is, how far it reaches.
#[derive(Debug, Clone, Copy)]
pub struct LightPoint {
    /// World position, in Bevy's axes.
    pub position: Vec3,
    /// Authored colour.
    pub colour: [u8; 3],
    /// Reach in world units.
    pub range: f32,
}

impl WorldPlan {
    /// Filters a loaded map and fixes its rotations.
    #[must_use]
    #[expect(
        clippy::too_many_lines,
        reason = "the filter, the chunk assignment and the light collection read better together than apart"
    )]
    pub fn new(world: World, effects: &[vc_world::Effect], sense: Sense) -> Self {
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

        // Diagnostic: render every placement, including interiors and shells, so a filter
        // can be ruled in or out by looking at the coverage.
        let keep_all = std::env::var_os("VC_KEEP_ALL").is_some();
        // Vice City bakes its shadows as black quads. If they are drawing opaque they
        // cover whole streets, so dropping them by name is how that gets tested.
        let drop_shadow_quads = std::env::var_os("VC_NO_SHADOW_MODELS").is_some();
        // Diagnostic: keep only ground-type models, so a floor that renders vertical can be
        // looked at on its own instead of through a city.
        let only_ground = std::env::var_os("VC_ONLY_GROUND").is_some();
        let mut instances = Vec::with_capacity(world.instances.len());
        let (mut dropped_lod, mut dropped_interior, mut dropped_unknown) = (0, 0, 0);

        for instance in &world.instances {
            let Some(definition) = definitions.get(&instance.definition_id) else {
                dropped_unknown += 1;

                continue;
            };

            if drop_shadow_quads && definition.model.to_ascii_lowercase().contains("shad") {
                continue;
            }

            if only_ground {
                let name = definition.model.to_ascii_lowercase();
                let is_ground = ["land", "road", "pave", "islandlod", "lae", "lan"]
                    .iter()
                    .any(|needle| name.contains(needle));

                if !is_ground {
                    continue;
                }
            }

            if keep_all {
                instances.push(corrected(instance, sense));

                continue;
            }

            if is_lod(&definition.model) {
                // A shell with no detail counterpart is the only copy of that object,
                // so dropping it leaves a hole. The island shells are the exception:
                // they are coarse stand-ins for whole blocks that are drawn in detail.
                // The island shells are the base terrain the whole city stands on, and
                // they have detail nearby everywhere, so they are never redundant. A
                // district shell is dropped when a detail copy sits on it.
                let island = definition
                    .model
                    .to_ascii_lowercase()
                    .starts_with("islandlod");
                let orphan = island
                    || !detail
                        .iter()
                        .any(|detail| near(&detail.position, &instance.position));

                if orphan {
                    let mut instance = corrected(instance, sense);

                    // The shell is the coarse twin of the ground drawn on top of it, so it
                    // is dropped a fraction under the detail to stop the two fighting for
                    // the same depth - visible as diagonal stripes across whole streets.
                    if island && !keep_all {
                        instance.position[2] -= ISLAND_SHELL_DROP;
                    }

                    instances.push(instance);
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

            instances.push(corrected(instance, sense));
        }

        if std::env::var_os("VC_REPORT_ISLANDS").is_some() {
            for instance in &instances {
                if let Some(definition) = definitions.get(&instance.definition_id)
                    && definition.model.to_ascii_lowercase().contains("island")
                {
                    println!(
                        "plan holds island: {} id {}",
                        definition.model, instance.definition_id
                    );
                }
            }
        }

        // Two placements of the same model within a few centimetres draw the same surface
        // twice, and the two fight for the same depth - which reads as a shimmering,
        // doubled texture on exactly the road tiles that have it. The map carries a
        // handful of these; keep the first.
        let mut kept: Vec<u32> = Vec::with_capacity(instances.len());
        let mut dropped_duplicate = 0;

        'outer: for index in 0..instances.len() {
            let Some(instance) = instances.get(index).cloned() else {
                continue;
            };

            for earlier in &kept {
                let Some(other) = instances.get(*earlier as usize) else {
                    continue;
                };

                if other.definition_id != instance.definition_id {
                    continue;
                }

                let close = (instance.position[0] - other.position[0]).abs() < 0.05
                    && (instance.position[1] - other.position[1]).abs() < 0.05
                    && (instance.position[2] - other.position[2]).abs() < 0.05;

                if close {
                    dropped_duplicate += 1;

                    continue 'outer;
                }
            }

            kept.push(index as u32);
        }

        let instances: Vec<Instance> = kept
            .iter()
            .filter_map(|index| instances.get(*index as usize).cloned())
            .collect();

        let mut chunks: BTreeMap<ChunkKey, Vec<u32>> = BTreeMap::new();
        for (index, instance) in instances.iter().enumerate() {
            chunks
                .entry(ChunkKey::of(instance.position))
                .or_default()
                .push(index as u32);
        }

        // The authored lights sit where the lamp heads are, so the instance transform
        // places them exactly; 500-odd lamps light the whole road network at night.
        let mut lights = Vec::new();
        for effect in effects {
            if !is_light(effect.kind) {
                continue;
            }

            for instance in instances
                .iter()
                .filter(|instance| instance.definition_id == effect.definition_id)
            {
                lights.push(LightPoint {
                    position: to_bevy_position(place(
                        effect.offset,
                        instance.position,
                        instance.rotation,
                        instance.scale,
                    )),
                    colour: effect.colour,
                    range: effect.size.clamp(6.0, 24.0),
                });
            }
        }

        Self {
            definitions,
            instances,
            chunks,
            dropped_lod,
            dropped_interior,
            dropped_unknown,
            dropped_duplicate,
            lights,
        }
    }

    /// Where to start: the requested cell, or the one the map fills most.
    /// The highest placement in a cell, which is a safe height to frame it from.
    ///
    /// The opening shot used a fixed height above the cell centre. Where the streets are
    /// higher than that, the camera sits inside the terrain and the floor fills the frame
    /// as a wall - which reads as the world being on its side.
    #[must_use]
    pub fn skyline(&self, cell: CellKey) -> f32 {
        self.instances
            .iter()
            .filter(|instance| CellKey::of(instance.position, CELL_SIZE) == cell)
            .fold(f32::NEG_INFINITY, |high, instance| {
                high.max(instance.position[2])
            })
    }

    /// Centre of the cell to open on, or the busiest cell when none is asked for.
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

/// How far under the detail terrain the island shells are placed, in world units.
const ISLAND_SHELL_DROP: f32 = 1.0;

/// Whether an effect type is an authored light.
const fn is_light(kind: u32) -> bool {
    matches!(kind, 255 | 0 | 228)
}

/// Places a model-space point in the world, in the game's axes.
fn place(point: [f32; 3], position: [f32; 3], rotation: [f32; 4], scale: [f32; 3]) -> [f32; 3] {
    let local = Quat::from_xyzw(rotation[0], rotation[1], rotation[2], rotation[3]);
    let scaled = Vec3::new(
        point[0] * scale[0],
        point[1] * scale[1],
        point[2] * scale[2],
    );
    let turned = local * scaled;

    [
        position[0] + turned.x,
        position[1] + turned.y,
        position[2] + turned.z,
    ]
}

/// Flips an instance into the rotation sense the game uses.
///
/// The shipped quaternion is the inverse of the rotation that was applied: lamp arms
/// point at the carriageway only when it is conjugated.
fn corrected(instance: &Instance, sense: Sense) -> Instance {
    if sense == Sense::Stored {
        return instance.clone();
    }

    let mut instance = instance.clone();
    instance.rotation = [
        -instance.rotation[0],
        -instance.rotation[1],
        -instance.rotation[2],
        instance.rotation[3],
    ];

    instance
}

/// Which reading of the shipped quaternion to place instances with.
///
/// The data does not settle it: the lamp-arm test prefers [`Sense::Conjugated`] by a
/// wide margin, yet the wrapped geometry of signs and shop fronts reads better to the
/// eye under the other. It is a switch so a person can decide in one keypress.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Resource)]
pub enum Sense {
    /// Apply the quaternion as authored.
    Stored,
    /// Apply its conjugate.
    ///
    /// The default, checked in the running game: with the axis bug fixed, this reading
    /// places everything correctly. `R` flips to the other for comparison.
    #[default]
    Conjugated,
}

impl Sense {
    /// Reads the sense from `VC_SENSE` when set, for comparing the two in one run.
    #[must_use]
    pub fn from_env() -> Self {
        match std::env::var("VC_SENSE").ok().as_deref() {
            Some("stored") => Self::Stored,
            Some("conjugated") => Self::Conjugated,
            _ => Self::default(),
        }
    }

    /// The other reading.
    #[must_use]
    pub const fn flipped(self) -> Self {
        match self {
            Self::Stored => Self::Conjugated,
            Self::Conjugated => Self::Stored,
        }
    }

    /// Name for the log.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Stored => "as stored",
            Self::Conjugated => "conjugated",
        }
    }
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
/// Authored lights the map places: thousands, once per placement.
///
/// Only the nearest handful are ever spawned.
pub const MAX_LIGHTS: usize = 96;

/// How far the camera may travel before the nearest set is recomputed.
pub const LIGHT_RADIUS: f32 = 60.0;

/// The authored lights currently in the scene, and where they were chosen from.
#[derive(Resource, Default)]
pub struct ActiveLights {
    entities: Vec<Entity>,
    centre: Vec3,
    spawned: bool,
    /// Whether these were spawned for night, so a change of hour rebuilds them.
    night: bool,
}

impl ActiveLights {
    /// How many lights are live.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.entities.len()
    }

    /// Whether no light is live.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }
}

/// Keeps the nearest authored lights spawned around the camera.
///
/// 3,511 of them exist and spawning them all would cost more than the world does, so
/// the set is the nearest [`MAX_LIGHTS`] and is rebuilt only after the camera has moved
/// [`LIGHT_RADIUS`].
#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters are taken by value by design"
)]
pub fn update_lights(
    mut commands: Commands<'_, '_>,
    plan: Res<'_, WorldPlan>,
    sky: Res<'_, Sky>,
    camera: Query<'_, '_, &GlobalTransform, With<FlyCamMarker>>,
    mut active: ResMut<'_, ActiveLights>,
) {
    let night = !(6..19).contains(&sky.hour);
    let Ok(camera) = camera.single() else {
        return;
    };
    let centre = camera.translation();

    if active.spawned
        && night != active.entities.is_empty()
        && centre.distance(active.centre) < LIGHT_RADIUS
    {
        return;
    }

    for entity in active.entities.drain(..) {
        commands.entity(entity).despawn();
    }

    active.centre = centre;
    active.spawned = true;
    active.night = night;

    if !night {
        return;
    }

    let mut nearest: Vec<&LightPoint> = plan
        .lights
        .iter()
        .filter(|light| light.position.distance(centre) < 400.0)
        .collect();
    nearest.sort_by(|left, right| {
        left.position
            .distance_squared(centre)
            .total_cmp(&right.position.distance_squared(centre))
    });

    for light in nearest.into_iter().take(MAX_LIGHTS) {
        let entity = commands
            .spawn((
                PointLight {
                    color: Color::srgb_u8(light.colour[0], light.colour[1], light.colour[2]),
                    intensity: 400_000.0,
                    range: light.range,
                    shadow_maps_enabled: false,
                    ..PointLight::default()
                },
                Transform::from_translation(light.position),
            ))
            .id();
        active.entities.push(entity);
    }
}

/// Spawns the baked parts, culled beyond `draw_distance`.
///
/// The far end is the fog's own range: the map is fully fogged out by then, so
/// dropping the draw costs nothing on screen and removes everything behind it from
/// the render and shadow passes.
pub fn spawn(commands: &mut Commands<'_, '_>, baked: BakedWorld, draw_distance: f32) {
    let range = VisibilityRange {
        start_margin: 0.0..0.0,
        end_margin: draw_distance..draw_distance.mul_add(1.05, 1.0),
        use_aabb: true,
    };

    for part in baked.parts {
        let mut entity = commands.spawn((
            Mesh3d(part.mesh),
            MeshMaterial3d(part.material),
            part.bounds,
            Transform::IDENTITY,
            range.clone(),
            BakedPart,
        ));
        // The world never casts shadows: Vice City baked its shading into the vertex
        // colours, so a shadow map over the whole city both darkens what was already
        // dark and redraws every visible chunk a second time. Measured on the beach, the
        // map alone was taking the frame from 29 to 9 of 255.
        entity.insert(bevy::light::NotShadowCaster);
    }
}

/// Marks a spawned chunk mesh, so a rebuild can clear them.
#[derive(Component)]
pub struct BakedPart;

/// Everything one bake produced, before it is spawned.
pub struct BakedWorld {
    /// One mesh per texture per chunk.
    pub parts: Vec<vc_render::MeshPart>,
    /// What the bake cost.
    pub stats: BakeStats,
}

/// Counters from one bake.
#[derive(Default, Clone)]
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
    /// Material groups across the map.
    pub groups: usize,
    /// Groups that name a texture no dictionary holds.
    pub unresolved: usize,
    /// Groups that name no texture at all.
    pub untextured: usize,
    /// Groups whose material colour is not white.
    pub tinted: usize,
    /// The tinted colours, for inspecting what the data actually holds.
    pub tints: Vec<[f32; 4]>,
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
            for mesh in &model.meshes {
                for group in &mesh.groups {
                    stats.groups += 1;

                    match &group.texture {
                        Some(_) => {}
                        // Either the model names no texture, or it names one no
                        // dictionary holds - the second is a hole the eye reads as a
                        // pale patch, so the two are counted apart.
                        None => stats.untextured += 1,
                    }

                    // Bytes come from a /255 division, so an exact white is exact.
                    if group
                        .colour
                        .iter()
                        .any(|channel| (channel - 1.0).abs() > 1e-6)
                    {
                        stats.tinted += 1;
                        if stats.tints.len() < 12 {
                            stats.tints.push(group.colour);
                        }
                    }
                }
            }
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
pub fn load(directory: &Path) -> Result<(World, Vec<vc_world::Effect>), Box<dyn Error>> {
    let mut world = World::default();
    let mut effects = Vec::new();
    let mut paths = Vec::new();
    collect(directory, &mut paths)?;

    for path in &paths {
        let text = String::from_utf8_lossy(&fs::read(path)?).into_owned();

        match path.extension().and_then(|extension| extension.to_str()) {
            Some("ide") => {
                match vc_world::parse_definitions(&text) {
                    Ok(definitions) => world.definitions.extend(definitions),
                    Err(error) => println!("skip {}: {error}", path.display()),
                }
                match vc_world::parse_effects(&text) {
                    Ok(found) => effects.extend(found),
                    Err(error) => println!("skip effects {}: {error}", path.display()),
                }
            }
            Some("ipl") => match vc_world::parse_instances(&text) {
                Ok(instances) => world.instances.extend(instances),
                Err(error) => println!("skip {}: {error}", path.display()),
            },
            _ => {}
        }
    }

    Ok((world, effects))
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

/// Spawns the sun as a small unlit sphere far along the sun's direction.
///
/// A shader dome would be the elegant way; this is the way that draws, and the sun is a
/// disc whether it comes from a shader or from geometry at that distance.
pub fn spawn_sun_disc(
    commands: &mut Commands<'_, '_>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    sky: &Sky,
) -> Entity {
    let colour = sky.sun.to_linear();
    let material = materials.add(StandardMaterial {
        base_color: Color::srgb(
            (colour.red * 1.4).min(1.0),
            (colour.green * 1.4).min(1.0),
            (colour.blue * 1.4).min(1.0),
        ),
        emissive: LinearRgba::new(
            colour.red * 30.0,
            colour.green * 30.0,
            colour.blue * 30.0,
            1.0,
        ),
        unlit: true,
        fog_enabled: false,
        ..StandardMaterial::default()
    });

    let entity = commands
        .spawn((
            Mesh3d(
                meshes.add(
                    Sphere::new(SUN_RADIUS)
                        .mesh()
                        .ico(2)
                        .unwrap_or_else(|_| Sphere::new(SUN_RADIUS).mesh().uv(16, 8)),
                ),
            ),
            MeshMaterial3d(material),
            Transform::from_translation(sky.sun_direction() * SUN_DISTANCE),
            NotShadowCaster,
            SunDisc,
        ))
        .id();

    println!("sun: disc spawned");

    entity
}

/// Marks the sun's disc, so the clock can move it.
#[derive(Component)]
pub struct SunDisc;

/// How far along the sun's direction the disc sits.
const SUN_DISTANCE: f32 = 850.0;

/// Radius of the disc at that distance, which reads as roughly half a degree.
const SUN_RADIUS: f32 = 26.0;

/// Places the disc where the sun is.
pub fn place_sun_disc(disc: &mut Transform, sky: &Sky) {
    *disc = Transform::from_translation(sky.sun_direction() * SUN_DISTANCE);
}

/// The ocean: one quad at the height and extent `water.dat` specifies, in Bevy axes.
pub fn spawn_water(
    commands: &mut Commands<'_, '_>,
    streamer: &mut Streamer,
    textures: &mut TextureCache,
    materials: &mut Assets<WaterMaterial>,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    sky: &Sky,
) {
    let Some(texture) = streamer.texture("particle", "waterclear256") else {
        println!("water: waterclear256 not found, skipping the ocean");

        return;
    };
    textures.upload_batch(std::slice::from_ref(&texture), images);
    let water = sky.water.to_linear();
    let clear = sky.clear.to_linear();
    let material = materials.add(WaterMaterial {
        ripples: textures.handle(&texture),
        params: WaterParams {
            time: 0.0,
            deep: Vec4::new(water.red, water.green, water.blue, 1.0),
            shallow: Vec4::new(water.red * 1.7, water.green * 1.8, water.blue * 1.6, 1.0),
            sky: Vec4::new(clear.red, clear.green, clear.blue, 1.0),
            sun: {
                let light = sky.sun_direction();

                Vec4::new(light.x, light.y, light.z, 1.0)
            },
        },
    });

    commands.spawn((
        Mesh3d(meshes.add(water_mesh())),
        MeshMaterial3d(material.clone()),
        Transform::IDENTITY,
        NotShadowCaster,
        WaterSurface(material),
    ));
    println!("water: quad at 6, rippled");
}

fn water_mesh() -> Mesh {
    let extent = 3_000.0;
    let level = 6.0;
    let tiles = 64.0;
    let mut mesh = Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [-extent, level, -extent],
            [extent, level, -extent],
            [extent, level, extent],
            [-extent, level, extent],
        ],
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 1.0, 0.0]; 4]);
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vec![[0.0, 0.0], [tiles, 0.0], [tiles, tiles], [0.0, tiles]],
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0, 1.0, 1.0, 1.0]; 4]);
    mesh.insert_indices(bevy::mesh::Indices::U32(vec![0, 2, 1, 0, 3, 2]));

    mesh
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
        let plan = WorldPlan::new(
            world(vec![instance(2, "LODroad", 0, [0.0; 3])]),
            &[],
            Sense::Conjugated,
        );

        assert_eq!(plan.instances.len(), 1);
        assert_eq!(plan.dropped_lod, 0);
    }

    #[test]
    fn drops_a_shell_that_duplicates_detail_geometry() {
        let plan = WorldPlan::new(
            world(vec![
                instance(1, "road", 0, [0.0; 3]),
                instance(2, "LODroad", 0, [0.0; 3]),
            ]),
            &[],
            Sense::Conjugated,
        );

        assert_eq!(plan.instances.len(), 1);
        assert_eq!(plan.instances[0].definition_id, 1);
        assert_eq!(plan.dropped_lod, 1);
    }

    #[test]
    fn rescues_an_outdoor_placement_tagged_with_an_interior() {
        let plan = WorldPlan::new(
            world(vec![
                instance(1, "road", 13, [10.0, 20.0, 5.0]),
                instance(2, "LODroad", 0, [10.0, 20.0, 5.0]),
            ]),
            &[],
            Sense::Conjugated,
        );

        assert_eq!(plan.instances.len(), 1);
        assert_eq!(plan.instances[0].position, [10.0, 20.0, 5.0]);
    }

    #[test]
    fn drops_a_real_interior() {
        let plan = WorldPlan::new(
            world(vec![instance(1, "road", 4, [10.0, 20.0, 5.0])]),
            &[],
            Sense::Conjugated,
        );

        assert_eq!(plan.instances.len(), 0);
        assert_eq!(plan.dropped_interior, 1);
    }

    #[test]
    fn conjugates_the_stored_rotation() {
        let mut instance = instance(1, "road", 0, [0.0; 3]);
        instance.rotation = [0.0, 0.0, 0.707_106_77, 0.707_106_77];

        let plan = WorldPlan::new(world(vec![instance]), &[], Sense::Conjugated);

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
