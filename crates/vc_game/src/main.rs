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

use bevy::input::mouse::MouseMotion;
use bevy::pbr::MeshMaterial3d;
use bevy::prelude::*;
use vc_render::{ModelPart, TextureCache, VcMaterial, VcRenderPlugin, upload_model};
use vc_stream::Streamer;
use vc_transport::{GameRoot, ImageArchive};
use vc_world::{CELL_SIZE, CellGrid, CellKey, ObjectDefinition, World};

/// Cells loaded in each direction from the camera.
const VIEW_RADIUS: i32 = 2;

/// Models cooked per frame, so streaming cannot stall the frame.
const MODELS_PER_FRAME: usize = 8;

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

    let grid = CellGrid::build(&world, CELL_SIZE);
    let start = grid.busiest().ok_or("the map holds no placements")?;
    let centre = start.centre(CELL_SIZE);

    let mut definitions = HashMap::with_capacity(world.definitions.len());
    for definition in world.definitions {
        let _ = definitions.insert(definition.id, definition);
    }

    println!(
        "cells: {} across the map, starting in ({}, {}) with {} placements",
        grid.cell_count(),
        start.x,
        start.y,
        grid.instances(start).len()
    );

    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "vc-rs — streaming Vice City".to_owned(),
                        ..default()
                    }),
                    ..default()
                })
                // Shaders live with the crate; the runtime cooks everything else in memory.
                .set(AssetPlugin {
                    file_path: "crates/vc_game/assets".to_owned(),
                    ..default()
                }),
        )
        .add_plugins(VcRenderPlugin)
        .insert_resource(ClearColor(Color::srgb(0.42, 0.56, 0.70)))
        .insert_resource(Stream {
            streamer: warmed(archive),
            definitions,
            instances: world.instances,
            grid,
            loaded: HashSet::new(),
            uploaded: HashMap::new(),
            spawned: 0,
            failed: 0,
        })
        .insert_resource(FlyCam {
            yaw: 0.0,
            pitch: -0.25,
        })
        .add_systems(Startup, move |mut commands: Commands<'_, '_>| {
            commands.spawn((
                Camera3d::default(),
                Transform::from_xyz(centre[0] - 120.0, 55.0, centre[1])
                    .looking_at(Vec3::new(centre[0], 5.0, centre[1]), Vec3::Y),
                FlyCamMarker,
            ));
        })
        .add_systems(Update, (fly_camera, stream_cells, report).chain())
        .run();

    Ok(())
}

#[derive(Resource)]
struct Stream {
    streamer: Streamer,
    definitions: HashMap<u32, ObjectDefinition>,
    instances: Vec<vc_world::Instance>,
    grid: CellGrid,
    loaded: HashSet<CellKey>,
    uploaded: HashMap<String, Vec<ModelPart>>,
    spawned: u64,
    failed: u64,
}

#[derive(Resource)]
struct FlyCam {
    yaw: f32,
    pitch: f32,
}

#[derive(Component)]
struct FlyCamMarker;

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters are taken by value by design"
)]
fn fly_camera(
    time: Res<'_, Time>,
    keys: Res<'_, ButtonInput<KeyCode>>,
    buttons: Res<'_, ButtonInput<MouseButton>>,
    mut motion: MessageReader<'_, '_, MouseMotion>,
    mut state: ResMut<'_, FlyCam>,
    mut camera: Query<'_, '_, &mut Transform, With<FlyCamMarker>>,
) {
    let speed = if keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight) {
        FLY_SPEED * SPRINT
    } else {
        FLY_SPEED
    };

    let looking = buttons.pressed(MouseButton::Left);

    for event in motion.read() {
        if looking {
            state.yaw = f32::mul_add(event.delta.x, -0.003, state.yaw);
            state.pitch = f32::mul_add(event.delta.y, -0.003, state.pitch).clamp(-1.5, 1.5);
        }
    }

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

fn stream_cells(
    mut commands: Commands<'_, '_>,
    mut stream: ResMut<'_, Stream>,
    camera: Query<'_, '_, &Transform, With<FlyCamMarker>>,
    mut meshes: ResMut<'_, Assets<Mesh>>,
    mut materials: ResMut<'_, Assets<VcMaterial>>,
    mut images: ResMut<'_, Assets<Image>>,
    mut textures: ResMut<'_, TextureCache>,
) {
    let Ok(transform) = camera.single() else {
        return;
    };

    let centre = CellKey::of(transform.translation.to_array(), stream.grid.size());
    let wanted = stream.grid.within(centre, VIEW_RADIUS);
    let mut budget = MODELS_PER_FRAME;

    for key in wanted {
        if stream.loaded.contains(&key) {
            continue;
        }

        let indices = stream.grid.instances(key).to_vec();
        let mut complete = true;

        for index in indices {
            let Some(instance) = stream.instances.get(index as usize).cloned() else {
                continue;
            };
            let Some(definition) = stream.definitions.get(&instance.definition_id).cloned() else {
                stream.failed += 1;

                continue;
            };

            let parts = if let Some(cached) = stream.uploaded.get(&definition.model) {
                cached.clone()
            } else {
                if budget == 0 {
                    complete = false;

                    break;
                }

                budget -= 1;

                if let Ok(model) = stream.streamer.model(&definition) {
                    let parts = upload_model(
                        &model,
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

            let position = Vec3::from(instance.position);
            let scale = Vec3::from(instance.scale);
            let rotation = Quat::from_xyzw(
                instance.rotation[0],
                instance.rotation[1],
                instance.rotation[2],
                instance.rotation[3],
            );

            let placed = Transform::from_translation(position)
                .with_rotation(rotation)
                .with_scale(scale);

            for part in &parts {
                commands.spawn((
                    Mesh3d(part.mesh.clone()),
                    MeshMaterial3d(part.material.clone()),
                    placed,
                ));
                stream.spawned += 1;
            }
        }

        if complete {
            let _ = stream.loaded.insert(key);
        }
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
