//! The diagnostics overlay.

use bevy::diagnostic::{Diagnostic, DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;

use crate::camera::FlyCamMarker;
use crate::world::{BakeStats, Sense, WorldPlan};

/// The baked scene's counters, shared with the overlay.
#[derive(Resource, Default, Clone)]
pub struct Stats(pub BakeStats);

/// Marks the overlay text.
#[derive(Component)]
pub struct Hud;

/// Spawns the overlay.
pub fn spawn(mut commands: Commands<'_, '_>) {
    commands.spawn((
        Text::new("vc-rs"),
        TextFont {
            font_size: bevy::text::FontSize::Px(14.0),
            ..default()
        },
        TextColor(Color::WHITE),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(10.0),
            left: Val::Px(10.0),
            ..default()
        },
        Hud,
    ));
}

/// Rewrites the overlay once per frame.
#[expect(
    clippy::needless_pass_by_value,
    clippy::too_many_arguments,
    reason = "Bevy system parameters are taken by value by design, and each is a distinct resource"
)]
pub fn update(
    diagnostics: Res<'_, DiagnosticsStore>,
    stats: Res<'_, Stats>,
    plan: Res<'_, WorldPlan>,
    sense: Res<'_, Sense>,
    camera: Query<'_, '_, &Transform, With<FlyCamMarker>>,
    mut hud: Query<'_, '_, &mut Text, With<Hud>>,
    mut nearest: Local<'_, (String, f32)>,
    mut tick: Local<'_, f32>,
    time: Res<'_, Time>,
) {
    let Ok(mut text) = hud.single_mut() else {
        return;
    };

    let smoothed = |id: &bevy::diagnostic::DiagnosticPath| {
        diagnostics
            .get(id)
            .and_then(Diagnostic::smoothed)
            .unwrap_or(0.0)
    };
    let (x, y, z) = camera.single().map_or((0.0, 0.0, 0.0), |transform| {
        let at = transform.translation;

        (at.x, -at.z, at.y)
    });

    // Naming the nearest placement turns a screenshot into a model name, which is the
    // difference between "something looks wrong" and a file to open.
    if time.elapsed_secs() - *tick > 0.5 {
        *tick = time.elapsed_secs();
        *nearest = plan
            .nearest_to([x, y, z])
            .unwrap_or_else(|| ("none".to_owned(), 0.0));
    }

    text.0 = format!(
        "{:5.1} fps  {:5.2} ms  sense {}\n{} chunks  {} draws  {} tris\n{} k verts  {} textures  {} failed\nat {:.0} {:.0} {:.0}\nnear {} at {:.0} m",
        smoothed(&FrameTimeDiagnosticsPlugin::FPS),
        smoothed(&FrameTimeDiagnosticsPlugin::FRAME_TIME),
        sense.label(),
        stats.0.chunks,
        stats.0.draws,
        stats.0.triangles,
        stats.0.vertices / 1_000,
        stats.0.textures,
        stats.0.failed,
        x,
        y,
        z,
        nearest.0,
        nearest.1,
    );
}
