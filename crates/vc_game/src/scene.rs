//! Camera and lighting setup shared by the game and the offscreen capture.

use bevy::camera::Hdr;
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::light::CascadeShadowConfigBuilder;
use bevy::pbr::{
    DistanceFog, FogFalloff, ScreenSpaceAmbientOcclusion, ScreenSpaceAmbientOcclusionQualityLevel,
};
use bevy::prelude::*;
use bevy::render::view::Msaa;

use crate::camera::FlyCamMarker;
use crate::sky::Sky;

/// Spawns the sun and the sky fill for one hour.
pub fn spawn_sun(commands: &mut Commands<'_, '_>, sky: &Sky) {
    let direction = sky.sun_direction();
    commands.spawn((
        DirectionalLight {
            color: sky.sun,
            illuminance: 10_000.0 * sky.power,
            shadow_maps_enabled: true,
            ..DirectionalLight::default()
        },
        // Two cascades over 250 m rather than four over 600: the world is prelit, so
        // shadows read as contact detail near the player, and the extra cascades cost a
        // full redraw of every visible chunk.
        CascadeShadowConfigBuilder {
            num_cascades: 2,
            minimum_distance: 1.0,
            maximum_distance: 250.0,
            first_cascade_far_bound: 40.0,
            overlap_proportion: 0.15,
        }
        .build(),
        Transform::from_rotation(Quat::from_rotation_arc(Vec3::NEG_Z, -direction)),
    ));
    commands.insert_resource(GlobalAmbientLight {
        color: sky.ambient,
        brightness: 600.0 * sky.power,
        affects_lightmapped_meshes: true,
    });
}

/// The world camera: HDR, the timecyc's fog and the opening orientation.
///
/// The `Camera` component is a parameter because the offscreen capture renders to an
/// image rather than to the window.
pub fn world_camera(eye: Vec3, target: Vec3, sky: &Sky, camera: Camera) -> impl Bundle {
    // Printed because a rolled camera reads exactly like "the textures are sideways", and
    // the basis is the only place in the frame where that can be seen from a log.
    let transform = Transform::from_translation(eye).looking_at(target, Vec3::Y);
    let rotation = transform.rotation;
    println!(
        "camera right {:?} up {:?} back {:?}",
        rotation * Vec3::X,
        rotation * Vec3::Y,
        rotation * Vec3::Z,
    );

    (
        Camera3d::default(),
        camera,
        Hdr,
        Tonemapping::AcesFitted,
        DebandDither::Enabled,
        // The city is dense, angular geometry, which is exactly what contact occlusion
        // is for; MSAA off pays for it at 1080p.
        Msaa::Off,
        ScreenSpaceAmbientOcclusion {
            quality_level: ScreenSpaceAmbientOcclusionQualityLevel::Medium,
            ..ScreenSpaceAmbientOcclusion::default()
        },
        Projection::from(PerspectiveProjection {
            near: 0.1,
            far: 12_000.0,
            ..PerspectiveProjection::default()
        }),
        DistanceFog {
            color: sky.clear,
            falloff: FogFalloff::Linear {
                start: sky.fog_start,
                end: sky.fog_end,
            },
            ..DistanceFog::default()
        },
        transform,
        FlyCamMarker,
    )
}
