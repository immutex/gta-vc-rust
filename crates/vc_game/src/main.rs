//! The vc-rs runtime: streams Vice City around a free-flying camera.
//!
//! Run it from the repository root, where `$VC_GAME_ROOT` (or `./gta-vc`) points at the install.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::diagnostic::DiagnosticsStore;
use bevy::input::mouse::MouseMotion;
use bevy::pbr::{DistanceFog, FogFalloff, MeshMaterial3d};
use bevy::prelude::*;
use vc_physics::{Placement, StaticWorld};
use vc_render::{
    Lighting, ModelPart, TextureCache, VcMaterial, VcRenderPlugin, to_bevy_position, upload_model,
};
use vc_stream::{Colliders, Streamer};
use vc_timecyc::Timecycle;
use vc_transport::{GameRoot, ImageArchive};
use vc_world::{CELL_SIZE, CellGrid, CellKey, ObjectDefinition, World};

/// Cells loaded in each direction from the camera.
const VIEW_RADIUS: i32 = 2;

/// Models cooked per frame, so streaming cannot stall the frame.
const MODELS_PER_FRAME: usize = 8;

/// Entities spawned per frame for the same reason: a cached model costs nothing to
/// cook but can still be hundreds of meshes.
const ENTITIES_PER_FRAME: usize = 512;

/// Gravity in game units per second squared. Vice City's own is about 7, which is
/// floaty; ten keeps the arc familiar without feeling like the moon.
const GRAVITY: f32 = 10.0;

/// Walking speed in game units per second.
const WALK_SPEED: f32 = 6.0;

/// Eye height above the player's feet.
const EYE_HEIGHT: f32 = 1.7;

/// Player radius against walls.
const PLAYER_RADIUS: f32 = 0.45;

/// Metres per second while flying.
const FLY_SPEED: f32 = 60.0;

/// Multiplier while holding shift.
const SPRINT: f32 = 4.0;

fn main() -> Result<(), Box<dyn Error>> {
    let root = GameRoot::from_env()?;
    let archive = ImageArchive::open(&root.image())?;
    let world = load_world(&root.path().join("data/maps"))?;

    println!(
        "world: {} definitions, {} placements",
        world.definitions.len(),
        world.instances.len()
    );

    let (world, definitions) = prepare(world);

    let grid = CellGrid::build(&world, CELL_SIZE);
    let start = grid.busiest().ok_or("the map holds no placements")?;
    let centre = start.centre(CELL_SIZE);

    println!(
        "cells: {} across the map, starting in ({}, {}) with {} placements",
        grid.cell_count(),
        start.x,
        start.y,
        grid.instances(start).len()
    );

    let sky = load_sky(&root.path().join("data/timecyc.dat"))?;

    println!(
        "sky: {} at {}h, far clip {:.0}, fog from {:.0}",
        sky.weather, sky.hour, sky.far_clip, sky.fog_start
    );

    // The baked shade is the original's sun, so it is kept at unity and the ambient
    // term does the lifting. 0.3 is roughly the fill a modern renderer would add and
    // is what makes the shadows readable without washing out the baked contrast.
    let fill = |channel: f32| channel.mul_add(1.6 / 255.0, 0.10);
    let sun = sky.directional[0]
        .max(sky.directional[1])
        .max(sky.directional[2])
        .mul_add(0.9 / 255.0, 0.55);
    let lighting = Lighting {
        ambient: Vec4::new(
            fill(sky.ambient[0]),
            fill(sky.ambient[1]),
            fill(sky.ambient[2]),
            1.0,
        ),
        sun: Vec4::splat(sun),
        floor: 0.35,
    };

    let collision = CollisionSet(Colliders::load(&archive));

    App::new()
        // The default asset root is `assets` next to the crate, which is where the
        // shader lives; overriding it doubles the path because Bevy already anchors
        // the root to the crate's manifest directory in a cargo build.
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "vc-rs — streaming Vice City".to_owned(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(VcRenderPlugin)
        .add_plugins(bevy::diagnostic::FrameTimeDiagnosticsPlugin::default())
        .add_plugins(bevy::diagnostic::EntityCountDiagnosticsPlugin::default())
        .insert_resource(ClearColor(sky.clear))
        .insert_resource(Stream {
            streamer: warmed(archive),
            definitions,
            instances: world.instances,
            grid,
            loaded: HashSet::new(),
            cursors: HashMap::new(),
            uploaded: HashMap::new(),
            physics: StaticWorld::new(25.0),
            colliders: HashMap::new(),
            spawned: 0,
            failed: 0,
        })
        .insert_resource(FlyCam::looking_at(
            to_bevy_position(centre) + Vec3::new(-120.0, 55.0, 0.0),
            to_bevy_position(centre),
        ))
        .insert_resource(sky)
        .insert_resource(Player {
            position: Vec3::new(centre[0], centre[1], 40.0),
            velocity: Vec3::ZERO,
            on_ground: false,
        })
        .insert_resource(Spawn(centre))
        .insert_resource(collision)
        .insert_resource(WorldLighting(lighting))
        .insert_resource(RotationMode::default())
        .add_systems(Startup, (setup_scene, spawn_hud))
        .add_systems(Update, update_hud)
        .add_systems(
            Update,
            (look, fly_camera, walk_player, rebuild, stream_cells, report).chain(),
        )
        .run();

    Ok(())
}

/// Collision models, wrapped so the streaming crate stays free of Bevy types.
#[derive(Resource)]
struct CollisionSet(Colliders);

/// Which sense instance rotations are applied in.
///
/// A quaternion and its conjugate describe the same rotation in opposite hands. The
/// data cannot say which the game meant, so this is a switch: press R and the world
/// respawns under the other convention, which settles it in one look.
#[derive(Resource)]
struct RotationMode {
    flipped: bool,
}

impl Default for RotationMode {
    /// Conjugated by default: `cargo xtask facing` places 2,421 authored effect anchors
    /// under both readings and the conjugated one lands nearer the road graph 66% of
    /// the time. The game's quaternions are the transposed sense of ours.
    fn default() -> Self {
        Self { flipped: true }
    }
}

/// Marks the diagnostics text so only it is rewritten each frame.
#[derive(Component)]
struct Hud;

/// The world's lighting, wrapped so systems can read it as one value.
#[derive(Resource, Clone, Copy)]
struct WorldLighting(Lighting);

/// Where the camera starts, in game space.
#[derive(Resource, Clone, Copy)]
struct Spawn([f32; 3]);

#[derive(Resource)]
struct Stream {
    streamer: Streamer,
    definitions: HashMap<u32, ObjectDefinition>,
    instances: Vec<vc_world::Instance>,
    grid: CellGrid,
    loaded: HashSet<CellKey>,
    cursors: HashMap<CellKey, usize>,
    uploaded: HashMap<String, Vec<ModelPart>>,
    physics: StaticWorld,
    colliders: HashMap<String, usize>,
    spawned: u64,
    failed: u64,
}

/// The player, in game coordinates: Z is up, as the map is authored.
#[derive(Resource)]
struct Player {
    position: Vec3,
    velocity: Vec3,
    on_ground: bool,
}

#[derive(Resource)]
struct FlyCam {
    yaw: f32,
    pitch: f32,
    /// False while the player is walking; the fly camera then leaves the view alone.
    walking: bool,
    /// Whether instance rotations are applied conjugated. See [`RotationMode`].
    flipped: bool,
}

impl FlyCam {
    /// Orientation that looks from `eye` towards `target`, so the opening view is the
    /// one the spawn was chosen for rather than a hard-coded angle.
    fn looking_at(eye: Vec3, target: Vec3) -> Self {
        let direction = (target - eye).normalize_or_zero();
        let horizontal = direction.x.hypot(direction.z);

        Self {
            yaw: f32::atan2(-direction.x, -direction.z),
            pitch: f32::atan2(direction.y, horizontal),
            walking: false,
            flipped: true,
        }
    }
}

#[derive(Component)]
struct FlyCamMarker;

/// Marks the entity every world instance hangs from.
#[derive(Component)]
struct WorldRootMarker;

/// The root entity itself, so streaming can parent to it.
#[derive(Resource, Clone, Copy)]
struct WorldRoot(Entity);

/// Turns the view and switches between walking and flying.
///
/// Look is its own system because both modes need it: the fly camera moves the view
/// itself, the walking player places the camera at their eye, but either way the mouse
/// has to turn it.
#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters are taken by value by design"
)]
fn look(
    keys: Res<'_, ButtonInput<KeyCode>>,
    buttons: Res<'_, ButtonInput<MouseButton>>,
    mut motion: MessageReader<'_, '_, MouseMotion>,
    mut state: ResMut<'_, FlyCam>,
) {
    if keys.just_pressed(KeyCode::KeyR) {
        state.flipped = !state.flipped;
        println!(
            "rotation sense: {}",
            if state.flipped {
                "conjugate"
            } else {
                "as stored"
            }
        );
    }

    if keys.just_pressed(KeyCode::KeyF) {
        state.walking = !state.walking;
        println!("mode: {}", if state.walking { "walk" } else { "fly" });
    }

    let looking = buttons.pressed(MouseButton::Left);

    for event in motion.read() {
        if looking {
            state.yaw = f32::mul_add(event.delta.x, -0.003, state.yaw);
            state.pitch = f32::mul_add(event.delta.y, -0.003, state.pitch).clamp(-1.5, 1.5);
        }
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters are taken by value by design"
)]
fn fly_camera(
    time: Res<'_, Time>,
    keys: Res<'_, ButtonInput<KeyCode>>,
    state: Res<'_, FlyCam>,
    mut camera: Query<'_, '_, &mut Transform, With<FlyCamMarker>>,
) {
    if state.walking {
        return;
    }

    let speed = if keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight) {
        FLY_SPEED * SPRINT
    } else {
        FLY_SPEED
    };

    let Ok(mut transform) = camera.single_mut() else {
        return;
    };

    let rotation = Quat::from_rotation_y(state.yaw) * Quat::from_rotation_x(state.pitch);
    transform.rotation = rotation;

    let forward = rotation * Vec3::NEG_Z;
    let right = rotation * Vec3::X;
    let mut direction = Vec3::ZERO;

    if keys.pressed(KeyCode::KeyW) {
        direction += forward;
    }
    if keys.pressed(KeyCode::KeyS) {
        direction -= forward;
    }
    if keys.pressed(KeyCode::KeyD) {
        direction += right;
    }
    if keys.pressed(KeyCode::KeyA) {
        direction -= right;
    }
    if keys.pressed(KeyCode::Space) {
        direction += Vec3::Y;
    }
    if keys.pressed(KeyCode::ControlLeft) {
        direction -= Vec3::Y;
    }

    if direction != Vec3::ZERO {
        transform.translation += direction.normalize() * speed * time.delta_secs();
    }
}

/// Keeps only what the runtime draws: the outside world, without LOD shells.
///
/// Interiors are a separate streaming problem. LOD models are dropped because every
/// one of them duplicates geometry that is also placed in detail, and drawing both is
/// what made surfaces overlap.
fn prepare(world: World) -> (World, HashMap<u32, ObjectDefinition>) {
    let mut definitions: HashMap<u32, ObjectDefinition> =
        HashMap::with_capacity(world.definitions.len());
    for definition in world.definitions {
        let _ = definitions.insert(definition.id, definition);
    }

    let instances = world
        .instances
        .into_iter()
        .filter(|instance| instance.interior <= 0)
        .filter(|instance| {
            definitions
                .get(&instance.definition_id)
                .is_none_or(|definition| !is_lod(&definition.model))
        })
        .collect();

    (
        World {
            definitions: Vec::new(),
            instances,
        },
        definitions,
    )
}

/// Rebuilds the world when the rotation sense changes.
///
/// Every mesh hangs off the world root, so despawning the root's children plus
/// forgetting which cells are loaded is enough: the streamer re-spawns them next frame
/// with the other convention, and the cooked models stay cached.
#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters are taken by value by design"
)]
fn rebuild(
    mut commands: Commands<'_, '_>,
    mode: Res<'_, RotationMode>,
    mut seen: Local<'_, bool>,
    mut stream: ResMut<'_, Stream>,
    root: Res<'_, WorldRoot>,
    children: Query<'_, '_, &ChildOf>,
) {
    if *seen == mode.flipped {
        return;
    }

    *seen = mode.flipped;

    if !stream.loaded.is_empty() || !stream.cursors.is_empty() {
        for child in &children {
            if child.parent() == root.0 {
                commands.entity(child.parent()).despawn();
            }
        }

        stream.loaded.clear();
        stream.cursors.clear();
        stream.spawned = 0;
        stream.physics = StaticWorld::new(25.0);
        stream.colliders.clear();
    }
}

/// Registers collision for one placed instance, if the install models it.
///
/// One collider per model, referenced by every instance of it, so the city's
/// collision costs a few megabytes rather than a few hundred.
fn place_collision(
    stream: &mut Stream,
    colliders: &CollisionSet,
    definition: &ObjectDefinition,
    instance: &vc_world::Instance,
) {
    let Some(collider) = colliders.0.get(&definition.model) else {
        return;
    };

    let index = if let Some(&known) = stream.colliders.get(&definition.model) {
        known
    } else {
        let added = stream.physics.add_collider(collider.clone());
        let _ = stream.colliders.insert(definition.model.clone(), added);

        added
    };
    let (min, max) = collider.bounds();

    stream.physics.place(
        index,
        Placement {
            position: instance.position,
            rotation: instance.rotation,
            scale: instance.scale,
        },
        (min, max),
    );
}

/// Streams the cells around the camera and spawns what they place.
#[expect(
    clippy::needless_pass_by_value,
    clippy::too_many_arguments,
    reason = "Bevy system parameters are taken by value by design, and each one is a distinct resource or asset collection"
)]
fn stream_cells(
    mut commands: Commands<'_, '_>,
    mut stream: ResMut<'_, Stream>,
    camera: Query<'_, '_, &Transform, With<FlyCamMarker>>,
    mut meshes: ResMut<'_, Assets<Mesh>>,
    mut materials: ResMut<'_, Assets<VcMaterial>>,
    mut images: ResMut<'_, Assets<Image>>,
    mut textures: ResMut<'_, TextureCache>,
    root: Res<'_, WorldRoot>,
    colliders: Res<'_, CollisionSet>,
    lighting: Res<'_, WorldLighting>,
    mode: Res<'_, RotationMode>,
) {
    let Ok(transform) = camera.single() else {
        return;
    };

    // The camera lives in Bevy space (Y up); the grid is keyed in game space (Z up),
    // where a cell is a patch of ground. Converting is the difference between
    // streaming the island and streaming a column of air.
    let at = transform.translation;
    let centre = CellKey::of([at.x, -at.z, at.y], stream.grid.size());
    let wanted = stream.grid.within(centre, VIEW_RADIUS);
    let mut budget = MODELS_PER_FRAME;
    let mut spawned_this_frame = 0;

    for key in wanted {
        if stream.loaded.contains(&key) {
            continue;
        }

        let indices = stream.grid.instances(key).to_vec();
        let mut complete = true;
        let start = stream.cursors.get(&key).copied().unwrap_or(0);

        for (position, index) in indices.iter().enumerate().skip(start) {
            let Some(instance) = stream.instances.get(*index as usize).cloned() else {
                continue;
            };
            let Some(definition) = stream.definitions.get(&instance.definition_id).cloned() else {
                stream.failed += 1;

                continue;
            };

            let parts = if let Some(cached) = stream.uploaded.get(&definition.model) {
                cached.clone()
            } else {
                if budget == 0 || spawned_this_frame >= ENTITIES_PER_FRAME {
                    complete = false;
                    let _ = stream.cursors.insert(key, position);

                    break;
                }

                budget -= 1;

                if let Ok(model) = stream.streamer.model(&definition) {
                    let parts = upload_model(
                        &model,
                        lighting.0,
                        &mut meshes,
                        &mut materials,
                        &mut images,
                        &mut textures,
                    );
                    let _ = stream
                        .uploaded
                        .insert(definition.model.clone(), parts.clone());

                    parts
                } else {
                    stream.failed += 1;

                    Vec::new()
                }
            };

            if parts.is_empty() {
                continue;
            }

            if spawned_this_frame + parts.len() > ENTITIES_PER_FRAME {
                complete = false;
                let _ = stream.cursors.insert(key, position);

                break;
            }

            // Straight from the map, in the game's own axes; the world root handles
            // the handed-up conversion for every mesh at once.
            let flipped = mode.flipped;
            let rotation = if flipped {
                Quat::from_xyzw(
                    -instance.rotation[0],
                    -instance.rotation[1],
                    -instance.rotation[2],
                    instance.rotation[3],
                )
            } else {
                Quat::from_xyzw(
                    instance.rotation[0],
                    instance.rotation[1],
                    instance.rotation[2],
                    instance.rotation[3],
                )
            };
            let placed = Transform::from_translation(Vec3::from(instance.position))
                .with_rotation(rotation)
                .with_scale(Vec3::from(instance.scale));

            for part in &parts {
                commands.spawn((
                    Mesh3d(part.mesh.clone()),
                    MeshMaterial3d(part.material.clone()),
                    placed,
                    ChildOf(root.0),
                ));
                stream.spawned += 1;
                spawned_this_frame += 1;
            }

            place_collision(&mut stream, &colliders, &definition, &instance);
        }

        if complete {
            let _ = stream.loaded.insert(key);
            let _ = stream.cursors.remove(&key);
        }
    }
}

/// Moves the player, in game coordinates, and puts the camera at their eye.
///
/// The collision queries are the ones `vc_physics` already provides: a downward ray
/// for the ground and a directional cast for walls. Where a cell has no collision at
/// all - beaches and props are not collidable in Vice City - the player is held at
/// the water line rather than falling out of the world.
#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters are taken by value by design"
)]
fn walk_player(
    time: Res<'_, Time>,
    keys: Res<'_, ButtonInput<KeyCode>>,
    state: Res<'_, FlyCam>,
    mut player: ResMut<'_, Player>,
    stream: Res<'_, Stream>,
    mut camera: Query<'_, '_, &mut Transform, With<FlyCamMarker>>,
) {
    if !state.walking {
        return;
    }

    let dt = time.delta_secs().min(0.1);
    let (sin, cos) = state.yaw.sin_cos();
    let forward = Vec3::new(-sin, cos, 0.0);
    let right = Vec3::new(cos, sin, 0.0);

    let mut desired = Vec3::ZERO;

    if keys.pressed(KeyCode::KeyW) {
        desired += forward;
    }
    if keys.pressed(KeyCode::KeyS) {
        desired -= forward;
    }
    if keys.pressed(KeyCode::KeyD) {
        desired += right;
    }
    if keys.pressed(KeyCode::KeyA) {
        desired -= right;
    }

    let speed = if keys.pressed(KeyCode::ShiftLeft) {
        WALK_SPEED * 2.0
    } else {
        WALK_SPEED
    };
    let horizontal = desired.normalize_or_zero() * speed;

    player.velocity.x = horizontal.x;
    player.velocity.y = horizontal.y;

    if player.on_ground && keys.just_pressed(KeyCode::Space) {
        player.velocity.z = 5.5;
        player.on_ground = false;
    }

    player.velocity.z = GRAVITY.mul_add(-dt, player.velocity.z);

    // Walls: stop short in the direction of travel.
    let step = horizontal.length() * dt;
    let mut moved = horizontal;

    if step > 0.0 {
        let origin = player.position + Vec3::Z * 1.0;
        let direction = horizontal.normalize();
        let reach = step + PLAYER_RADIUS;
        let hit = stream
            .physics
            .cast(origin.to_array(), direction.to_array(), reach);

        if hit < reach {
            moved = Vec3::ZERO;
        }
    }

    let fall = Vec3::Z * (player.velocity.z * dt);
    player.position += moved * dt + fall;

    // Ground: snap when the surface is within a step, otherwise keep falling.
    let probe = player.position + Vec3::Z * 0.5;
    let ground = stream.physics.ground(probe.to_array(), 4.0);
    player.on_ground = false;

    if let Some(height) = ground
        && player.position.z <= height + 0.4
    {
        player.position.z = height;
        player.velocity.z = 0.0;
        player.on_ground = true;
    }

    // No collision under this spot: hold at the water line rather than dropping out.
    if ground.is_none() && player.position.z < 6.0 {
        player.position.z = 6.0;
        player.velocity.z = 0.0;
        player.on_ground = true;
    }

    let eye = player.position + Vec3::Z * EYE_HEIGHT;
    let rotation = Quat::from_rotation_y(state.yaw) * Quat::from_rotation_x(state.pitch);

    if let Ok(mut transform) = camera.single_mut() {
        transform.translation = to_bevy_position(eye.to_array());
        transform.rotation = rotation;
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters are taken by value by design"
)]
fn report(time: Res<'_, Time>, stream: Res<'_, Stream>, mut last: Local<'_, f32>) {
    if time.elapsed_secs() - *last < 2.0 {
        return;
    }

    *last = time.elapsed_secs();

    let stats = stream.streamer.stats();
    println!(
        "cells {}/{} | entities {} | models {} | textures {} | dictionaries {} | failed {}",
        stream.loaded.len(),
        stream.grid.cell_count(),
        stream.spawned,
        stats.models,
        stats.textures,
        stats.dictionaries,
        stream.failed
    );
}

/// The sky, fog and water for one hour of one weather type.
#[derive(Resource)]
struct Sky {
    weather: String,
    hour: usize,
    clear: Color,
    fog_start: f32,
    far_clip: f32,
    water: [f32; 4],
    ambient: [f32; 3],
    directional: [f32; 3],
}

fn load_sky(path: &Path) -> Result<Sky, Box<dyn Error>> {
    let text = String::from_utf8_lossy(&fs::read(path)?).into_owned();
    let table = Timecycle::parse(&text)?;
    let weather = table
        .weathers
        .first()
        .ok_or("timecyc.dat holds no weather types")?;
    let hour_index = 12;
    let hour = weather
        .hours
        .get(hour_index)
        .ok_or("timecyc.dat is missing noon")?;

    Ok(Sky {
        weather: weather.name.clone(),
        hour: hour_index,
        clear: linear_to_color(hour.sky_bottom),
        fog_start: hour.fog_start,
        far_clip: hour.far_clip,
        water: hour.water,
        ambient: hour.ambient,
        directional: hour.directional,
    })
}

/// Timecyc colours are already linear, which is what Bevy's fog wants.
const fn linear_to_color(rgb: [f32; 3]) -> Color {
    Color::LinearRgba(LinearRgba::new(rgb[0], rgb[1], rgb[2], 1.0))
}

/// Whether a model is a level-of-detail shell.
///
/// Vice City marks them by name only: 1,082 of the 4,131 models start with `lod` or
/// end in `LOD`, and no other model contains that substring. There is no flag bit in
/// the IDE or the IPL that separates them.
fn is_lod(model: &str) -> bool {
    model.to_ascii_lowercase().contains("lod")
}

#[expect(
    clippy::needless_pass_by_value,
    clippy::too_many_arguments,
    reason = "Bevy system parameters are taken by value by design, and each is a distinct resource or asset collection"
)]
fn setup_scene(
    mut commands: Commands<'_, '_>,
    spawn: Res<'_, Spawn>,
    sky: Res<'_, Sky>,
    mut stream: ResMut<'_, Stream>,
    mut meshes: ResMut<'_, Assets<Mesh>>,
    mut materials: ResMut<'_, Assets<VcMaterial>>,
    mut images: ResMut<'_, Assets<Image>>,
    mut textures: ResMut<'_, TextureCache>,
) {
    // Every model's vertices are in the game's Z-up axes, so the whole world hangs off
    // one rotated root instead of converting each vertex or each instance. The camera
    // stays outside the hierarchy and converts its own position, which keeps the two
    // consistent. GlobalTransform and Visibility must be present explicitly: a parent
    // without them makes every child log B0004 and stops propagation.
    let root = commands
        .spawn((
            Transform::from_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
            GlobalTransform::default(),
            Visibility::default(),
            WorldRootMarker,
        ))
        .id();
    commands.insert_resource(WorldRoot(root));

    spawn_water(
        &mut commands,
        &mut stream.streamer,
        &mut meshes,
        &mut materials,
        &mut images,
        &mut textures,
        root,
        &sky,
    );

    commands.spawn((
        Camera3d::default(),
        // Vice City had no HDR pipeline: the texture colour is the colour.
        Tonemapping::None,
        Projection::from(PerspectiveProjection {
            near: 0.1,
            // The fog ends as far as 2000 units out; clipping sooner would cut
            // geometry before it dissolved into the sky.
            far: 3000.0,
            ..default()
        }),
        DistanceFog {
            color: sky.clear,
            falloff: FogFalloff::Linear {
                start: sky.fog_start,
                end: sky.far_clip,
            },
            ..default()
        },
        Transform::from_translation(to_bevy_position(spawn.0) + Vec3::new(-120.0, 55.0, 0.0))
            .looking_at(to_bevy_position(spawn.0), Vec3::Y),
        FlyCamMarker,
    ));
}

/// Top-left readout: frame rate, frame time and what the streamer is holding.
///
/// GPU utilisation is not portable, and without it a single "GPU %" would be a
/// guess; frame time is the honest number, and it is what the budget is spent in.
fn spawn_hud(mut commands: Commands<'_, '_>) {
    commands.spawn((
        Text::new("vc-rs"),
        TextFont {
            font_size: bevy::text::FontSize::Px(13.0),
            ..default()
        },
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(8.0),
            left: Val::Px(8.0),
            ..default()
        },
        Hud,
    ));
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters are taken by value by design"
)]
fn update_hud(
    diagnostics: Res<'_, DiagnosticsStore>,
    stream: Res<'_, Stream>,
    mut hud: Query<'_, '_, &mut Text, With<Hud>>,
) {
    let Ok(mut text) = hud.single_mut() else {
        return;
    };

    let fps = diagnostics
        .get(&bevy::diagnostic::FrameTimeDiagnosticsPlugin::FPS)
        .and_then(bevy::diagnostic::Diagnostic::smoothed)
        .unwrap_or(0.0);
    let frame_ms = diagnostics
        .get(&bevy::diagnostic::FrameTimeDiagnosticsPlugin::FRAME_TIME)
        .and_then(bevy::diagnostic::Diagnostic::smoothed)
        .unwrap_or(0.0);
    let stats = stream.streamer.stats();

    text.0 = format!(
        "{fps:5.1} fps  {frame_ms:5.2} ms\ncells {}/{}  entities {}\nmodels {}  textures {}  failed {}",
        stream.loaded.len(),
        stream.grid.cell_count(),
        stream.spawned,
        stats.models,
        stats.textures,
        stream.failed,
    );
}

/// Draws the ocean as one quad at the height and colour the game's data specifies.
///
/// `data/water.dat` holds a single rectangle - x and y spanning ±3000 at z = 6.0 - and
/// the texture comes from `particle.txd`. The tint is `timecyc.dat`'s water column, so
/// the sea changes with weather and hour like everything else.
#[expect(
    clippy::too_many_arguments,
    reason = "each argument is a distinct Bevy resource or asset collection"
)]
fn spawn_water(
    commands: &mut Commands<'_, '_>,
    stream: &mut Streamer,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<VcMaterial>,
    images: &mut Assets<Image>,
    textures: &mut TextureCache,
    root: Entity,
    sky: &Sky,
) {
    let Some(texture) = stream.texture("particle", "waterclear256") else {
        println!("water: waterclear256 not found, skipping the ocean");

        return;
    };

    let extent = 3000.0;
    let level = 6.0;
    let tiles = 64.0;

    let mut mesh = Mesh::new(
        bevy::render::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    );
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [-extent, -extent, level],
            [extent, -extent, level],
            [extent, extent, level],
            [-extent, extent, level],
        ],
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0, 0.0, 1.0]; 4]);
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vec![[0.0, 0.0], [tiles, 0.0], [tiles, tiles], [0.0, tiles]],
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0, 1.0, 1.0, 1.0]; 4]);
    mesh.insert_indices(bevy::render::mesh::Indices::U32(vec![0, 1, 2, 0, 2, 3]));

    let water = sky.water;
    let material = materials.add(VcMaterial {
        diffuse: Some(textures.handle(&texture, images)),
        uniforms: vc_render::VcUniforms {
            tint: Vec4::new(water[0] / 255.0, water[1] / 255.0, water[2] / 255.0, 1.0),
            cutout: 0.0,
            floor: 0.0,
            ambient: Vec4::ZERO,
            sun: Vec4::ONE,
        },
        cutout: false,
    });

    let entity = commands
        .spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(material)))
        .id();
    commands.entity(entity).insert(ChildOf(root));

    println!("water: {extent:.0} unit quad at z = {level}, tinted from timecyc");
}

fn warmed(archive: ImageArchive) -> Streamer {
    let mut streamer = Streamer::open(archive);
    let started = std::time::Instant::now();
    let indexed = streamer.warm_textures();

    println!(
        "textures: {indexed} indexed in {:.1}s",
        started.elapsed().as_secs_f32()
    );

    streamer
}

fn load_world(directory: &Path) -> Result<World, Box<dyn Error>> {
    let mut world = World::default();
    let mut paths = Vec::new();
    collect(directory, &mut paths)?;

    for path in &paths {
        let bytes = fs::read(path)?;
        let text = String::from_utf8_lossy(&bytes);

        match path.extension().and_then(|found| found.to_str()) {
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
            path.extension().and_then(|found| found.to_str()),
            Some("ide" | "ipl")
        ) {
            out.push(path);
        }
    }

    Ok(())
}
