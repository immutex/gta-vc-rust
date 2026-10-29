//! Camera and lighting setup shared by the game and the offscreen capture.

use bevy::camera::Hdr;
use bevy::core_pipeline::tonemapping::{DebandDither, Tonemapping};
use bevy::light::CascadeShadowConfigBuilder;
use bevy::pbr::{
    DistanceFog, FogFalloff, ScreenSpaceAmbientOcclusion, ScreenSpaceAmbientOcclusionQualityLevel,
};
use bevy::post_process::bloom::Bloom;
use bevy::post_process::dof::DepthOfField;
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
            // Diagnostic switch: the world's shade is already baked into its vertex
            // colours, so shadows can be turned off to see what they were doing.
            shadow_maps_enabled: std::env::var_os("VC_NO_SHADOWS").is_none(),
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
    // Sky fill, and it has to be strong. The map's shade is baked into its vertices, so a
    // surface facing away from the sun carries a prelit value near zero and no amount of
    // sunlight reaches it - at 600 the black patches on the roads stayed pure black. At
    // 3,000 the darkest surface in a measured frame is 11 of 255 rather than 0, which is
    // the difference between a shadow and a hole.
    let fill = std::env::var("VC_AMBIENT")
        .ok()
        .and_then(|value| value.parse::<f32>().ok())
        .unwrap_or(3_000.0);
    commands.insert_resource(GlobalAmbientLight {
        color: sky.ambient,
        brightness: fill * sky.power,
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
        // Four samples. The city is all thin edges - railings, wires, palm fronds - and
        // without it they crawl badly as the camera moves.
        if std::env::var_os("VC_NO_AA").is_some() {
            Msaa::Off
        } else {
            Msaa::Sample4
        },
        // Contact occlusion: the city is dense angular geometry, which is what it is for.
        ScreenSpaceAmbientOcclusion {
            quality_level: ScreenSpaceAmbientOcclusionQualityLevel::Medium,
            ..ScreenSpaceAmbientOcclusion::default()
        },
        // Light bleeding from the sky and the street lights. Natural is the subtle of
        // Bevy's presets; anything stronger fogs the whole skyline.
        Bloom::NATURAL,
        // Depth of field, kept gentle: the far skyline softens, the street does not.
        DepthOfField {
            focal_distance: 80.0,
            sensor_height: 0.015,
            aperture_f_stops: 0.25,
            ..DepthOfField::default()
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
