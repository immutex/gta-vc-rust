//! The sea: one quad, rippled rather than painted.

use bevy::asset::Asset;
use bevy::material::AlphaMode;
use bevy::pbr::Material;
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;

/// Path of the water shader, relative to the asset root.
pub const WATER_SHADER: &str = "shaders/water.wesl";

/// What the water shader reads.
#[derive(Clone, Copy, Debug, ShaderType)]
pub struct WaterParams {
    /// Seconds, driving the drifts.
    pub time: f32,
    /// Colour where the water is deep.
    pub deep: Vec4,
    /// Colour where it is thin.
    pub shallow: Vec4,
    /// The sky, for the reflected tint.
    pub sky: Vec4,
    /// Sun direction, its strength in `w`.
    pub sun: Vec4,
}

/// Rippled water.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct WaterMaterial {
    /// The authored caustic texture, used as the ripple field.
    #[texture(0)]
    #[sampler(1)]
    pub ripples: Option<Handle<Image>>,
    /// Everything the shader needs besides the texture.
    #[uniform(2)]
    pub params: WaterParams,
}

impl Material for WaterMaterial {
    fn fragment_shader() -> ShaderRef {
        WATER_SHADER.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Opaque
    }
}

/// Marks the sea, so its drift can advance.
#[derive(Component, Clone)]
pub struct WaterSurface(pub Handle<WaterMaterial>);

/// Registers the water material and its animation.
pub struct WaterPlugin;

impl Plugin for WaterPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<WaterMaterial>::default())
            .add_systems(Update, advance);
    }
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters are taken by value by design"
)]
fn advance(
    time: Res<'_, Time>,
    surfaces: Query<'_, '_, &WaterSurface>,
    mut materials: ResMut<'_, Assets<WaterMaterial>>,
) {
    for surface in &surfaces {
        if let Some(mut material) = materials.get_mut(&surface.0) {
            material.params.time = time.elapsed_secs() % 1_000.0;
        }
    }
}
