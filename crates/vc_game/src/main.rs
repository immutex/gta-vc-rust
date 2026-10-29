//! The vc-rs runtime: the whole of Vice City, resident and physically lit.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

use std::error::Error;

use bevy::prelude::*;
use vc_render::water::{WaterMaterial, WaterPlugin};
use vc_render::{MaterialCache, TextureCache, to_bevy_position};
use vc_stream::{Colliders, Streamer};
use vc_transport::{GameRoot, ImageArchive};
use vc_world::CellKey;

use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use vc_game::camera::{FlyCam, Physics, Player};
use vc_game::hud::{self, Stats};
use vc_game::scene;
use vc_game::sky::Sky;
use vc_game::world::{self, WorldPlan};

/// Eye height of the opening shot above the spawn cell.
const OPENING_HEIGHT: f32 = 55.0;
/// How far back the opening shot starts.
const OPENING_BACK: f32 = 120.0;

fn main() -> Result<(), Box<dyn Error>> {
    let options = Options::parse();
    let root = GameRoot::from_env()?;
    let archive = ImageArchive::open(&root.image())?;
    let mut streamer = Streamer::open(archive);
    let indexed = streamer.warm_textures();
    println!("textures: {indexed} indexed");

    let (world, effects) = world::load(&root.path().join("data/maps"))?;
    let sense = world::Sense::from_env();
    println!("rotation sense: {}", sense.label());
    let plan = WorldPlan::new(world.clone(), &effects, sense);
    println!(
        "world: {} placements ({} LOD, {} interior, {} duplicate, {} unknown dropped)",
        plan.instances.len(),
        plan.dropped_lod,
        plan.dropped_interior,
        plan.dropped_duplicate,
        plan.dropped_unknown,
    );

    let sky = Sky::load(&root.path().join("data/timecyc.dat"), options.hour)?;
    println!(
        "sky: {} at {}h, fog {:.0}..{:.0}, sun power {:.2}",
        sky.weather, sky.hour, sky.fog_start, sky.fog_end, sky.power
    );

    let colliders = ColliderSet(Colliders::load(streamer.archive()));
    let cell = options.cell.unwrap_or(CellKey { x: 1, y: 1 });
    let start = plan.start(Some(cell));
    let target = to_bevy_position(start);
    let eye = target + Vec3::new(-OPENING_BACK, OPENING_HEIGHT, 0.0);
    let camera = FlyCam::looking_at(eye, target);

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "vc-rs — Vice City, resident".to_owned(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(bevy::diagnostic::FrameTimeDiagnosticsPlugin::default())
        .add_plugins(WaterPlugin)
        .insert_resource(ClearColor(sky.clear))
        .insert_resource(sky)
        .insert_resource(plan)
        .insert_resource(Source(streamer))
        .insert_resource(colliders)
        .insert_resource(Physics(vc_physics::StaticWorld::new(world::PHYSICS_BUCKET)))
        .insert_resource(camera)
        .insert_resource(Player {
            position: Vec3::new(start[0], start[1], 40.0),
            velocity: Vec3::ZERO,
            on_ground: false,
        })
        .insert_resource(StartShot { eye, target })
        .insert_resource(StartCell(cell))
        .init_resource::<TextureCache>()
        .init_resource::<MaterialCache>()
        .init_resource::<Stats>()
        .init_resource::<world::ActiveLights>()
        .insert_resource(RawWorld(world))
        .insert_resource(sense)
        .insert_resource(Clock {
            table: vc_game::sky::read(&root.path().join("data/timecyc.dat"))?,
            hour: options.hour as f32,
            speed: std::env::var("VC_TIME_SCALE")
                .ok()
                .and_then(|value| value.parse::<f32>().ok())
                .unwrap_or(1.0 / 60.0),
        })
        .insert_resource(Effects(effects))
        .add_systems(
            Startup,
            (setup_world, setup_lighting, setup_camera, hud::spawn),
        )
        // PostStartup, not Startup: a projection inserted into the spawn queue is replaced
        // by the camera's own required component, leaving Bevy's default 1,000-unit far
        // plane - which clips everything beyond a kilometre, sky included.
        .add_systems(PostStartup, apply_projection)
        .add_systems(
            Update,
            (
                vc_game::camera::look,
                advance_clock,
                world::update_lights,
                grab_frame,
                flip_sense,
                vc_game::camera::fly,
                vc_game::camera::walk,
                hud::update,
            )
                .chain(),
        )
        .run();

    Ok(())
}

/// Writes what the player is looking at to `shot-<x>-<y>.png` on F12.
///
/// The offscreen capture frames a fixed view; this frames *theirs*, which is what a report
/// about a specific surface needs - the camera, the place and whatever is in front of it.
#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters are taken by value by design"
)]
fn grab_frame(
    mut commands: Commands<'_, '_>,
    keys: Res<'_, ButtonInput<KeyCode>>,
    camera: Query<'_, '_, &GlobalTransform, With<vc_game::camera::FlyCamMarker>>,
    grid: Res<'_, StartCell>,
) {
    if !keys.just_pressed(KeyCode::F12) {
        return;
    }

    let at = camera
        .single()
        .map(GlobalTransform::translation)
        .unwrap_or_default();
    let cell = CellKey::of([at.x, -at.z, at.y], vc_world::CELL_SIZE);
    let path = format!("shot-{}-{}.png", cell.x, cell.y);

    println!("capturing {path}");
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path));

    let _ = grid;
}

/// Re-bakes the world under the other rotation sense when R is pressed.
///
/// Rebaking is 2.3 s and keeps every cooked model and uploaded texture, which is far
/// cheaper than guessing which reading the art was authored in.
#[expect(
    clippy::too_many_arguments,
    clippy::needless_pass_by_value,
    reason = "each argument is a distinct resource the bake needs; Bevy passes them by value"
)]
fn flip_sense(
    mut commands: Commands<'_, '_>,
    keys: Res<'_, ButtonInput<KeyCode>>,
    mut sense: ResMut<'_, world::Sense>,
    raw: Res<'_, RawWorld>,
    effects: Res<'_, Effects>,
    mut plan: ResMut<'_, WorldPlan>,
    mut source: ResMut<'_, Source>,
    colliders: Res<'_, ColliderSet>,
    mut physics: ResMut<'_, Physics>,
    mut textures: ResMut<'_, TextureCache>,
    mut materials: ResMut<'_, MaterialCache>,
    mut meshes: ResMut<'_, Assets<Mesh>>,
    mut standard: ResMut<'_, Assets<StandardMaterial>>,
    mut images: ResMut<'_, Assets<Image>>,
    sky: Res<'_, Sky>,
    parts: Query<'_, '_, Entity, With<world::BakedPart>>,
) {
    if !keys.just_pressed(KeyCode::KeyR) {
        return;
    }

    *sense = sense.flipped();
    println!("rotation sense: {} (rebuilding)", sense.label());

    for entity in &parts {
        commands.entity(entity).despawn();
    }

    *plan = WorldPlan::new(raw.0.clone(), &effects.0, *sense);
    *physics = Physics(vc_physics::StaticWorld::new(world::PHYSICS_BUCKET));

    let baked = world::bake(
        &plan,
        &mut source.0,
        &colliders.0,
        &mut physics.0,
        &mut textures,
        &mut materials,
        &mut meshes,
        &mut standard,
        &mut images,
    );
    world::spawn_sun_disc(&mut commands, &mut meshes, &mut standard, &sky);
    world::spawn(&mut commands, baked, sky.fog_end);
}

/// Where the opening shot looks from and at.
#[derive(Resource, Clone, Copy)]
struct StartShot {
    eye: Vec3,
    target: Vec3,
}

/// The loaded map before filtering, kept so the plan can be rebuilt under the other
/// rotation sense without re-reading the disk.
#[derive(Resource)]
struct RawWorld(vc_world::World);

/// Command-line start cell and hour.
#[derive(Resource)]
struct Source(Streamer);

/// Collision models, wrapped so the ECS can hold them.
#[derive(Resource)]
struct ColliderSet(Colliders);

/// The `2dfx` effects, kept for the same reason.
#[derive(Resource)]
struct Effects(Vec<vc_world::Effect>);

/// The world clock, driving the timecyc and everything downstream of it.
#[derive(Resource)]
struct Clock {
    table: vc_timecyc::Timecycle,
    hour: f32,
    speed: f32,
}

/// The cell the opening shot is framed on.
#[derive(Resource, Clone, Copy)]
struct StartCell(CellKey);

/// Command-line start cell and hour.
struct Options {
    cell: Option<CellKey>,
    hour: usize,
}

impl Options {
    fn parse() -> Self {
        let mut arguments = std::env::args().skip(1);
        let x = arguments.next().and_then(|value| value.parse::<i32>().ok());
        let y = arguments.next().and_then(|value| value.parse::<i32>().ok());
        let hour = arguments
            .next()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(12)
            .min(23);

        Self {
            // Cell (1, 1) is where the opening shot was framed and checked: a street with
            // buildings, palms and water beyond. The busiest cell, which is what this used
            // to fall back on, is open sea - the game opened looking at nothing.
            cell: Some(
                x.zip(y)
                    .map_or(CellKey { x: 1, y: 1 }, |(x, y)| CellKey { x, y }),
            ),
            hour,
        }
    }
}

/// Bakes every chunk, uploads its textures and registers collision.
#[expect(
    clippy::needless_pass_by_value,
    clippy::too_many_arguments,
    reason = "Bevy system parameters are taken by value by design, and each is a distinct resource or asset collection"
)]
fn setup_world(
    mut commands: Commands<'_, '_>,
    mut source: ResMut<'_, Source>,
    plan: Res<'_, WorldPlan>,
    cell: Res<'_, StartCell>,
    mut water: ResMut<'_, Assets<WaterMaterial>>,
    mut start: ResMut<'_, StartShot>,
    colliders: Res<'_, ColliderSet>,
    mut physics: ResMut<'_, Physics>,
    mut textures: ResMut<'_, TextureCache>,
    mut materials: ResMut<'_, MaterialCache>,
    mut meshes: ResMut<'_, Assets<Mesh>>,
    mut standard: ResMut<'_, Assets<StandardMaterial>>,
    mut images: ResMut<'_, Assets<Image>>,
    sky: Res<'_, Sky>,
    mut stats: ResMut<'_, Stats>,
) {
    let started = std::time::Instant::now();
    let baked = world::bake(
        &plan,
        &mut source.0,
        &colliders.0,
        &mut physics.0,
        &mut textures,
        &mut materials,
        &mut meshes,
        &mut standard,
        &mut images,
    );

    println!(
        "bake: {} chunks, {} draws, {} triangles, {} vertices, {} textures, {} failed in {:.1}s",
        baked.stats.chunks,
        baked.stats.draws,
        baked.stats.triangles,
        baked.stats.vertices,
        baked.stats.textures,
        baked.stats.failed,
        started.elapsed().as_secs_f32(),
    );
    stats.0 = baked.stats.clone();

    // Frame the opening shot above whatever is actually in the cell, not a fixed height
    // above its centre - otherwise the camera can start inside the terrain.
    let high = plan.skyline(cell.0).max(0.0);

    if high + 20.0 > start.eye.y {
        let lift = high + 20.0 - start.eye.y;
        start.eye.y += lift;
        start.target.y = lift.mul_add(0.5, start.target.y);
    }

    world::spawn(&mut commands, baked, sky.fog_end);

    world::spawn_water(
        &mut commands,
        &mut source.0,
        &mut textures,
        &mut water,
        &mut meshes,
        &mut images,
        &sky,
    );
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters are taken by value by design"
)]
fn setup_lighting(mut commands: Commands<'_, '_>, sky: Res<'_, Sky>) {
    scene::spawn_sun(&mut commands, &sky);
}

/// Advances the world clock and carries every light with it.
///
/// An hour of game time per minute of real time by default, `VC_TIME_SCALE` to change the
/// pace and `VC_TIME_PAUSED` to hold the hour still.
#[expect(
    clippy::too_many_arguments,
    clippy::needless_pass_by_value,
    reason = "each argument is a distinct resource the clock drives; Bevy passes them by value"
)]
fn advance_clock(
    time: Res<'_, Time>,
    mut clock: ResMut<'_, Clock>,
    mut sky: ResMut<'_, Sky>,
    mut clear: ResMut<'_, ClearColor>,
    mut ambient: ResMut<'_, GlobalAmbientLight>,
    mut sun: Query<'_, '_, (&mut DirectionalLight, &mut Transform), With<scene::SunLight>>,
    mut camera: Query<'_, '_, &mut DistanceFog>,
    mut disc: Query<'_, '_, &mut Transform, With<world::SunDisc>>,
) {
    if std::env::var_os("VC_TIME_PAUSED").is_some() {
        return;
    }

    let before = clock.hour.floor() as usize;
    clock.hour = time.delta_secs().mul_add(clock.speed, clock.hour) % 24.0;
    let after = clock.hour.floor() as usize;

    if before == after {
        return;
    }

    let Ok(fresh) = Sky::at(&clock.table, after) else {
        return;
    };

    clear.0 = fresh.clear;
    ambient.color = fresh.ambient;
    ambient.brightness = ambient_fill() * fresh.power;

    for (mut light, mut transform) in &mut sun {
        scene::place_sun(&mut light, &mut transform, &fresh);
    }

    for mut transform in &mut disc {
        world::place_sun_disc(&mut transform, &fresh);
    }

    // Fog follows the hour too: dawn and dusk pull it in, noon pushes it out.
    for mut fog in &mut camera {
        fog.color = fresh.clear;
        fog.falloff = FogFalloff::Linear {
            start: fresh.fog_start,
            end: fresh.fog_end,
        };
    }

    *sky = fresh;
    println!("hour {}: {:?}", sky.hour, sky.weather);
}

/// The ambient fill, with `VC_AMBIENT` to tune it.
fn ambient_fill() -> f32 {
    std::env::var("VC_AMBIENT")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .unwrap_or(3_000.0)
}

fn apply_projection(mut commands: Commands<'_, '_>, camera: Query<'_, '_, Entity, With<Camera3d>>) {
    for entity in &camera {
        commands.entity(entity).insert(scene::world_projection());
    }
    println!("camera: far plane 12000");
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters are taken by value by design"
)]
fn setup_camera(mut commands: Commands<'_, '_>, sky: Res<'_, Sky>, shot: Res<'_, StartShot>) {
    let camera = commands
        .spawn(scene::world_camera(
            shot.eye,
            shot.target,
            &sky,
            Camera::default(),
        ))
        .id();
    commands.entity(camera).insert(scene::world_projection());
}
