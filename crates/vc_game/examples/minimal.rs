//! Minimal headless screenshot: camera, cube, light.
use std::error::Error;
use std::time::Duration;

use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::ExitCondition;
use bevy::winit::WinitPlugin;

#[derive(Resource)] struct Target(Handle<Image>);
#[derive(Resource)] struct Path(String);

fn main() -> Result<(), Box<dyn Error>> {
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
    .add_plugins(bevy::app::ScheduleRunnerPlugin::run_loop(Duration::from_millis(16)))
    .insert_resource(ClearColor(Color::srgb(0.1, 0.2, 0.4)))
    
    .insert_resource(Path("/tmp/vc_min.png".to_owned()))
    .add_systems(Startup, setup)
    .add_systems(Update, capture);
    app.run();
    Ok(())
}

fn setup(
    mut commands: Commands<'_, '_>,
    mut images: ResMut<'_, Assets<Image>>,
    mut meshes: ResMut<'_, Assets<Mesh>>,
    mut materials: ResMut<'_, Assets<StandardMaterial>>,
) {
    let mut image = Image::new_fill(
        Extent3d { width: 320, height: 240, depth_or_array_layers: 1 },
        TextureDimension::D2,
        &[0, 0, 0, 255],
        TextureFormat::Bgra8UnormSrgb,
        RenderAssetUsages::default(),
    );
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC | TextureUsages::RENDER_ATTACHMENT;
    let handle = images.add(image);
    commands.insert_resource(Target(handle.clone()));
    commands.spawn((
        Camera3d::default(),
        Camera::default(),
        RenderTarget::Image(handle.into()),
        Transform::from_xyz(0.0, 2.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    commands.spawn((
        DirectionalLight { shadow_maps_enabled: true, ..DirectionalLight::default() },
        Transform::from_rotation(Quat::from_rotation_x(-0.7)),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(2.0, 2.0, 2.0))),
        MeshMaterial3d(materials.add(StandardMaterial { base_color: Color::srgb(1.0, 0.2, 0.1), ..StandardMaterial::default() })),
        Transform::IDENTITY,
    ));
    commands.insert_resource(GlobalAmbientLight { color: Color::WHITE, brightness: 300.0, affects_lightmapped_meshes: true });
}

fn capture(mut commands: Commands<'_, '_>, mut frames: Local<'_, u32>, target: Res<'_, Target>, path: Res<'_, Path>, mut exit: MessageWriter<'_, AppExit>) {
    *frames += 1;
    if *frames == 40 {
        commands.spawn(Screenshot::image(target.0.clone())).observe(save_to_disk(path.0.clone()));
    }
    if *frames > 40 && std::path::Path::new(&path.0).exists() {
        exit.write(AppExit::Success);
    }
}
