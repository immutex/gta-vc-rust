//! The vc-rs runtime: opens a cooked scene and draws it.
//!
//! Run `cargo xtask cook <model> --txd <txd>` first, then `cargo run -p vc_game`.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

use std::path::PathBuf;

use anyhow::{Result, anyhow, ensure};
use bevy::pbr::MeshMaterial3d;
use bevy::prelude::*;
use vc_cook::Scene;
use vc_render::{VcMaterial, VcRenderPlugin, upload_scene};

/// Default artifact, produced by `cargo xtask cook landstal.dff --txd generic.txd`.
const DEFAULT_SCENE: &str = "vc-artifacts/cooked/landstal.vcck";

fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .map_or_else(|| PathBuf::from(DEFAULT_SCENE), PathBuf::from);

    let bytes = std::fs::read(&path).map_err(|error| {
        anyhow!(
            "failed to read `{}`: {error} — run `cargo xtask cook landstal.dff --txd generic.txd` first",
            path.display()
        )
    })?;

    let scene = Scene::decode(&bytes)
        .map_err(|error| anyhow!("failed to decode the cooked scene: {error}"))?;

    ensure!(
        !scene.meshes.is_empty(),
        "`{}` holds no meshes to draw",
        path.display()
    );

    println!(
        "loaded {} with {} meshes, {} triangles, {} textures",
        path.display(),
        scene.meshes.len(),
        scene.triangle_count(),
        scene.textures.len()
    );

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "vc-rs".to_owned(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(VcRenderPlugin)
        .insert_resource(LoadedScene(scene))
        .add_systems(Startup, setup)
        .run();

    Ok(())
}

#[derive(Resource)]
struct LoadedScene(Scene);

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters are taken by value by design; they are smart pointers, not data"
)]
fn setup(
    mut commands: Commands<'_, '_>,
    loaded: Res<'_, LoadedScene>,
    mut meshes: ResMut<'_, Assets<Mesh>>,
    mut images: ResMut<'_, Assets<Image>>,
    mut materials: ResMut<'_, Assets<VcMaterial>>,
) {
    let uploaded = upload_scene(&loaded.0, &mut meshes, &mut images, &mut materials);
    let (min, max) = uploaded.bounds;

    let centre = (Vec3::from(min) + Vec3::from(max)) / 2.0;
    let extent = (Vec3::from(max) - Vec3::from(min)).length().max(1.0);

    if uploaded.parts.is_empty() {
        eprintln!("the cooked scene produced no drawable parts");

        return;
    }

    for (mesh, material) in uploaded.parts {
        commands.spawn((Mesh3d(mesh), MeshMaterial3d(material), Transform::default()));
    }

    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(centre + Vec3::new(extent, extent * 0.6, extent))
            .looking_at(centre, Vec3::Y),
    ));
}
