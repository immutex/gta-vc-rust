//! Materials and scene upload for the vc-rs runtime.
//!
//! The material is deliberately small: a diffuse texture, a tint, and the
//! `RenderWare` state that changes silhouettes rather than shading. Prelit vertex
//! colours, dual texture and the per-triangle material split come next; this is
//! the smallest thing that can put a cooked mesh on screen.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

use bevy::image::ImageSampler;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::mesh::MeshVertexBufferLayoutRef;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError, TextureFormat,
};
use bevy::shader::ShaderRef;

/// Path of the material shader, relative to the asset root.
pub const SHADER_PATH: &str = "shaders/vc_material.wesl";

/// A RenderWare-flavoured material: one texture, one tint.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct VcMaterial {
    /// Diffuse texture.
    #[texture(0)]
    #[sampler(1)]
    pub diffuse: Option<Handle<Image>>,
    /// Colour multiplier, used for the flat-colour case.
    #[uniform(2)]
    pub tint: LinearRgba,
}

impl Default for VcMaterial {
    fn default() -> Self {
        Self {
            diffuse: None,
            tint: LinearRgba::WHITE,
        }
    }
}

impl Material for VcMaterial {
    // The default mesh vertex shader, because our shader only has a fragment entry.

    fn fragment_shader() -> ShaderRef {
        SHADER_PATH.into()
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(2),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];

        Ok(())
    }
}

/// Registers the material with the app.
pub struct VcRenderPlugin;

impl Plugin for VcRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<VcMaterial>::default());
    }
}

/// A scene's GPU-side assets, ready to spawn.
pub struct UploadedScene {
    /// One mesh per cooked mesh, with the material to draw it.
    pub parts: Vec<(Handle<Mesh>, Handle<VcMaterial>)>,
    /// Bounding box of the whole scene, as `[min, max]`.
    pub bounds: ([f32; 3], [f32; 3]),
}

/// Uploads a cooked scene into Bevy assets.
///
/// Textures are uploaded in the format the cook produced, so block-compressed
/// data is never decoded on the CPU.
pub fn upload_scene(
    scene: &vc_cook::Scene,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<VcMaterial>,
) -> UploadedScene {
    let textures: Vec<Handle<Image>> = scene
        .textures
        .iter()
        .map(|texture| images.add(upload_texture(texture)))
        .collect();

    let fallback = materials.add(VcMaterial::default());
    let mut parts = Vec::with_capacity(scene.meshes.len());
    let mut bounds = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);

    for cooked in &scene.meshes {
        if cooked.vertices.is_empty() || cooked.indices.is_empty() {
            continue;
        }

        let mut mesh = Mesh::new(
            bevy::render::mesh::PrimitiveTopology::TriangleList,
            bevy::asset::RenderAssetUsages::default(),
        );

        let positions: Vec<[f32; 3]> = cooked.vertices.iter().map(|v| v.position).collect();
        let normals: Vec<[f32; 3]> = cooked.vertices.iter().map(|v| v.normal).collect();
        let uvs: Vec<[f32; 2]> = cooked.vertices.iter().map(|v| v.uv[0]).collect();

        for position in &positions {
            for (axis, value) in position.iter().enumerate() {
                bounds.0[axis] = bounds.0[axis].min(*value);
                bounds.1[axis] = bounds.1[axis].max(*value);
            }
        }

        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
        mesh.insert_indices(bevy::render::mesh::Indices::U32(cooked.indices.clone()));

        let material = materials.add(VcMaterial {
            diffuse: textures.first().cloned(),
            tint: LinearRgba::WHITE,
        });

        parts.push((meshes.add(mesh), material));
    }

    if parts.is_empty() {
        bounds = ([0.0; 3], [0.0; 3]);
    }

    let _ = fallback;

    UploadedScene { parts, bounds }
}

fn upload_texture(texture: &vc_cook::CookedTexture) -> Image {
    let format = match texture.format {
        vc_cook::TextureFormat::Bc1 => TextureFormat::Bc1RgbaUnorm,
        vc_cook::TextureFormat::Bc3 => TextureFormat::Bc3RgbaUnorm,
        vc_cook::TextureFormat::Rgba8 => TextureFormat::Rgba8UnormSrgb,
    };

    let level = &texture.levels[0];
    let mut image = Image::new(
        bevy::render::render_resource::Extent3d {
            width: u32::from(texture.width),
            height: u32::from(texture.height),
            depth_or_array_layers: 1,
        },
        bevy::render::render_resource::TextureDimension::D2,
        level.data.clone(),
        format,
        bevy::asset::RenderAssetUsages::default(),
    );

    // Anisotropy needs linear filtering everywhere, which is what we want anyway.
    image.sampler = ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor::linear());

    image
}
