//! Renders one frame of the world offscreen and writes it to a PNG.
//!
//! Useful on machines without a windowing GPU: Vulkan's lavapipe is enough.
//! `VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json cargo run --release -p vc_game --example shot`

use std::error::Error;
use std::time::Duration;

use bevy::app::ScheduleRunnerPlugin;
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::ExitCondition;
use bevy::winit::WinitPlugin;
use vc_game::camera::{FlyCam, Physics, Player};
use vc_game::scene;
use vc_game::sky::Sky;
use vc_game::world::{self, WorldPlan};
use vc_render::{MaterialCache, TextureCache, to_bevy_position};
use vc_stream::{Colliders, Streamer};
use vc_transport::{GameRoot, ImageArchive};

/// Frames to render before the capture: assets finish preparing in the first frame or
/// two, and a software rasteriser is slow, so this stays small on purpose.
const WARMUP_FRAMES: u32 = 40;

#[derive(Resource, Clone)]
struct Target(Handle<Image>);

#[derive(Resource)]
struct Shot {
    path: String,
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut arguments = std::env::args().skip(1);
    let path = arguments.next().unwrap_or_else(|| "shot.png".to_owned());
    // Printed as an absolute path so the file can be found from anywhere, and so the
    // command works with no arguments at all.
    let absolute = std::env::current_dir()
        .map_or_else(|_| std::path::PathBuf::from(&path), |dir| dir.join(&path));
    println!("writing {}", absolute.display());
    let path = absolute.display().to_string();
    let cell_x = arguments.next().and_then(|value| value.parse::<i32>().ok());
    let cell_y = arguments.next().and_then(|value| value.parse::<i32>().ok());
    let hour = arguments
        .next()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(12)
        .min(23);
    let height = arguments
        .next()
        .and_then(|value| value.parse::<f32>().ok())
        .unwrap_or(55.0);
    let back = arguments
        .next()
        .and_then(|value| value.parse::<f32>().ok())
        .unwrap_or(120.0);

    let root = GameRoot::from_env()?;
    let archive = ImageArchive::open(&root.image())?;
    let mut streamer = Streamer::open(archive);
    let _ = streamer.warm_textures();

    let (world, effects) = world::load(&root.path().join("data/maps"))?;
    let plan = WorldPlan::new(world, &effects, vc_game::world::Sense::default());
    let sky = Sky::load(&root.path().join("data/timecyc.dat"), hour)?;
    let colliders = Colliders::load(streamer.archive());
    let start = plan.start(cell_x.zip(cell_y).map(|(x, y)| vc_world::CellKey { x, y }));
    let target = to_bevy_position(start);
    let eye = target + Vec3::new(-back, height, 0.0);

    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: None,
                exit_condition: ExitCondition::DontExit,
                close_when_requested: false,
                ..WindowPlugin::default()
            })
            .disable::<WinitPlugin>(),
    )
    .add_plugins(ScheduleRunnerPlugin::run_loop(Duration::from_millis(16)))
    .insert_resource(ClearColor(sky.clear))
    .insert_resource(sky)
    .insert_resource(plan)
    .insert_resource(StreamSource(streamer))
    .insert_resource(ColliderSet(colliders))
    .insert_resource(Physics(vc_physics::StaticWorld::new(world::PHYSICS_BUCKET)))
    .insert_resource(FlyCam::looking_at(eye, target))
    .insert_resource(Player {
        position: Vec3::new(start[0], start[1], 40.0),
        velocity: Vec3::ZERO,
        on_ground: false,
    })
    .insert_resource(StartShot { eye, target })
    .insert_resource(Shot { path })
    .init_resource::<TextureCache>()
    .init_resource::<MaterialCache>()
    .add_systems(Startup, (setup_world, setup_sun, setup_camera))
    .add_systems(Update, capture);

    app.run();

    Ok(())
}

#[derive(Resource)]
struct StreamSource(Streamer);

#[derive(Resource)]
struct ColliderSet(Colliders);

#[derive(Resource, Clone, Copy)]
struct StartShot {
    eye: Vec3,
    target: Vec3,
}

#[expect(
    clippy::needless_pass_by_value,
    clippy::too_many_arguments,
    reason = "Bevy system parameters are taken by value by design"
)]
fn setup_world(
    mut commands: Commands<'_, '_>,
    sky: Res<'_, Sky>,
    mut source: ResMut<'_, StreamSource>,
    plan: Res<'_, WorldPlan>,
    colliders: Res<'_, ColliderSet>,
    mut physics: ResMut<'_, Physics>,
    mut textures: ResMut<'_, TextureCache>,
    mut materials: ResMut<'_, MaterialCache>,
    mut meshes: ResMut<'_, Assets<Mesh>>,
    mut standard: ResMut<'_, Assets<StandardMaterial>>,
    mut images: ResMut<'_, Assets<Image>>,
) {
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
        "bake: {} draws, {} triangles in {} chunks",
        baked.stats.draws, baked.stats.triangles, baked.stats.chunks
    );
    println!("parts: {}", baked.parts.len());
    if let Some(part) = baked.parts.first() {
        println!("first part bounds: {:?}", part.bounds);
    }
    world::spawn(&mut commands, baked, sky.fog_end);
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters are taken by value by design"
)]
fn setup_sun(mut commands: Commands<'_, '_>, sky: Res<'_, Sky>) {
    scene::spawn_sun(&mut commands, &sky);
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters are taken by value by design"
)]
fn setup_camera(
    mut commands: Commands<'_, '_>,
    sky: Res<'_, Sky>,
    shot: Res<'_, StartShot>,
    mut images: ResMut<'_, Assets<Image>>,
    mut standard: ResMut<'_, Assets<StandardMaterial>>,
    mut meshes: ResMut<'_, Assets<Mesh>>,
) {
    // The camera is HDR, so its target has to be a float texture; an LDR target with an
    // HDR view renders nothing at all, which is a black PNG rather than a warning.
    let mut image = Image::new_target_texture(800, 450, TextureFormat::Rgba16Float, None);
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    let handle = images.add(image);
    commands.insert_resource(Target(handle.clone()));
    commands.spawn((
        scene::world_camera(shot.eye, shot.target, &sky, Camera::default()),
        RenderTarget::Image(handle.into()),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(20.0, 20.0, 20.0))),
        MeshMaterial3d(standard.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.0, 0.0),
            ..StandardMaterial::default()
        })),
        Transform::from_translation(shot.target + Vec3::Y * 10.0),
    ));
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters are taken by value by design"
)]
fn capture(
    mut commands: Commands<'_, '_>,
    mut frames: Local<'_, u32>,
    target: Res<'_, Target>,
    shot: Res<'_, Shot>,
    mut exit: MessageWriter<'_, AppExit>,
) {
    *frames += 1;

    if *frames == WARMUP_FRAMES {
        println!("capturing to {}", shot.path);
        commands
            .spawn(Screenshot::image(target.0.clone()))
            .observe(save_to_disk(shot.path.clone()));
    }

    // The save observer writes the file on this thread; once it is there the readback
    // is complete and the app can stop.
    let written = std::path::Path::new(&shot.path);

    if *frames > WARMUP_FRAMES && written.exists() {
        println!(
            "wrote {} ({} bytes)",
            shot.path,
            std::fs::metadata(written).map_or(0, |meta| meta.len())
        );
        exit.write(AppExit::Success);
    }

    // Without this the capture waits forever, which reads like a hang rather than a
    // failure; naming the frame it gave up on says which half of the path broke.
    if *frames > WARMUP_FRAMES * 40 {
        eprintln!("never wrote {} after {} frames", shot.path, *frames);
        exit.write(AppExit::Error(
            std::num::NonZero::new(1).unwrap_or_else(|| unreachable!()),
        ));
    }
}
