//! The vc-rs runtime: the whole of Vice City, resident and physically lit.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

use std::error::Error;

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use vc_render::{MaterialCache, TextureCache, to_bevy_position, water_material};
use vc_stream::{Colliders, Streamer};
use vc_transport::{GameRoot, ImageArchive};
use vc_world::CellKey;

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
    let sense = world::Sense::default();
    println!("rotation sense: {}", sense.label());
    let plan = WorldPlan::new(world.clone(), &effects, sense);
    println!(
        "world: {} placements ({} LOD, {} interior, {} unknown dropped)",
        plan.instances.len(),
        plan.dropped_lod,
        plan.dropped_interior,
        plan.dropped_unknown,
    );

    let sky = Sky::load(&root.path().join("data/timecyc.dat"), options.hour)?;
    // Cells differ a lot in how good a first view they give: the shoreline ones read as a
    // city floating over water, and the densest are interiors. These were checked by
    // rendering them.
    if options.cell.is_none() {
        println!("cell 0 1 gives a street view, 1 1 the waterfront, 0 -5 the beach");
    }
    println!(
        "sky: {} at {}h, fog {:.0}..{:.0}, sun power {:.2}",
        sky.weather, sky.hour, sky.fog_start, sky.fog_end, sky.power
    );

    let colliders = ColliderSet(Colliders::load(streamer.archive()));
    let start = plan.start(options.cell);
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
        .init_resource::<TextureCache>()
        .init_resource::<MaterialCache>()
        .init_resource::<Stats>()
        .init_resource::<world::ActiveLights>()
        .insert_resource(RawWorld(world))
        .insert_resource(sense)
        .insert_resource(Effects(effects))
        .add_systems(
            Startup,
            (setup_world, setup_lighting, setup_camera, hud::spawn),
        )
        .add_systems(
            Update,
            (
                vc_game::camera::look,
                world::update_lights,
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
            cell: x.zip(y).map(|(x, y)| CellKey { x, y }),
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

    world::spawn(&mut commands, baked, sky.fog_end);

    spawn_water(
        &mut commands,
        &mut source.0,
        &mut textures,
        &mut standard,
        &mut meshes,
        &mut images,
        &sky,
    );
}

fn spawn_water(
    commands: &mut Commands<'_, '_>,
    streamer: &mut Streamer,
    textures: &mut TextureCache,
    materials: &mut Assets<StandardMaterial>,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    sky: &Sky,
) {
    let Some(texture) = streamer.texture("particle", "waterclear256") else {
        println!("water: waterclear256 not found, skipping the ocean");

        return;
    };
    textures.upload_batch(std::slice::from_ref(&texture), images);
    let material = water_material(
        sky.water,
        textures.handle(&texture).unwrap_or_default(),
        materials,
    );

    commands.spawn((
        Mesh3d(meshes.add(water_mesh())),
        MeshMaterial3d(material),
        Transform::IDENTITY,
        NotShadowCaster,
    ));
    println!("water: quad at 6, tinted from timecyc");
}

/// The ocean: one quad at the height and extent `water.dat` specifies, in Bevy axes.
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

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters are taken by value by design"
)]
fn setup_lighting(mut commands: Commands<'_, '_>, sky: Res<'_, Sky>) {
    scene::spawn_sun(&mut commands, &sky);
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters are taken by value by design"
)]
fn setup_camera(mut commands: Commands<'_, '_>, sky: Res<'_, Sky>, shot: Res<'_, StartShot>) {
    commands.spawn(scene::world_camera(
        shot.eye,
        shot.target,
        &sky,
        Camera::default(),
    ));
}
