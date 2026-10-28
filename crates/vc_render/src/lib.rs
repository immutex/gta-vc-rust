//! Materials and upload for the vc-rs runtime.
//!
//! The material is deliberately small: a diffuse texture, a tint, and the `RenderWare` state that
//! changes silhouettes rather than shading. Prelit vertex colours and lighting come later; this is
//! the smallest thing that puts a real Vice City model on screen looking like itself.
//!
//! Uploads are deduplicated by identity: a texture shared by fifty models is uploaded once, and a
//! geometry with one material slot becomes one mesh without copying its vertex buffer.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

use std::collections::HashMap;
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, RenderPipelineDescriptor, SpecializedMeshPipelineError,
    TextureDimension, TextureFormat,
};
use bevy::shader::ShaderRef;
use vc_cook::CookedTexture;
use vc_stream::CookedModel;

/// Path of the material shader, relative to the asset root.
pub const SHADER_PATH: &str = "shaders/vc_material.wesl";

/// A `RenderWare`-flavoured material: one texture, one tint.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct VcMaterial {
    /// Diffuse texture.
    #[texture(0)]
    #[sampler(1)]
    pub diffuse: Option<Handle<Image>>,
    /// Colour multiplier.
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

/// Keeps one GPU texture per cooked texture, however many models ask for it.
#[derive(Default, Resource)]
pub struct TextureCache {
    uploaded: HashMap<usize, Handle<Image>>,
}

impl TextureCache {
    /// Uploads a texture once and hands back the shared handle.
    pub fn handle(
        &mut self,
        texture: &Arc<CookedTexture>,
        images: &mut Assets<Image>,
    ) -> Handle<Image> {
        let key = Arc::as_ptr(texture) as usize;

        if let Some(handle) = self.uploaded.get(&key) {
            return handle.clone();
        }

        let handle = images.add(upload_texture(texture));
        let _ = self.uploaded.insert(key, handle.clone());

        handle
    }

    /// How many textures have been uploaded.
    pub fn len(&self) -> usize {
        self.uploaded.len()
    }

    /// Whether nothing has been uploaded yet.
    pub fn is_empty(&self) -> bool {
        self.uploaded.is_empty()
    }
}

/// One drawable piece of a model.
#[derive(Clone)]
pub struct ModelPart {
    /// Mesh to draw.
    pub mesh: Handle<Mesh>,
    /// Material to draw it with.
    pub material: Handle<VcMaterial>,
}

/// Uploads a cooked model into Bevy assets.
pub fn upload_model(
    model: &CookedModel,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<VcMaterial>,
    images: &mut Assets<Image>,
    textures: &mut TextureCache,
) -> Vec<ModelPart> {
    let mut parts = Vec::new();

    for mesh in &model.meshes {
        if mesh.vertices.is_empty() || mesh.groups.is_empty() {
            continue;
        }

        let positions: Vec<[f32; 3]> = mesh.vertices.iter().map(|vertex| vertex.position).collect();
        let normals: Vec<[f32; 3]> = mesh.vertices.iter().map(|vertex| vertex.normal).collect();
        let uvs: Vec<[f32; 2]> = mesh.vertices.iter().map(|vertex| vertex.uv[0]).collect();

        for group in &mesh.groups {
            let mut built = Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::default(),
            );
            built.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions.clone());
            built.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals.clone());
            built.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs.clone());
            built.insert_indices(Indices::U32(group.indices.clone()));

            let diffuse = group
                .texture
                .as_ref()
                .map(|texture| textures.handle(texture, images));

            parts.push(ModelPart {
                mesh: meshes.add(built),
                material: materials.add(VcMaterial {
                    diffuse,
                    tint: LinearRgba::WHITE,
                }),
            });
        }
    }

    parts
}

fn upload_texture(texture: &CookedTexture) -> Image {
    let format = match texture.format {
        vc_cook::TextureFormat::Bc1 => TextureFormat::Bc1RgbaUnorm,
        vc_cook::TextureFormat::Bc3 => TextureFormat::Bc3RgbaUnorm,
        vc_cook::TextureFormat::Rgba8 => TextureFormat::Rgba8UnormSrgb,
    };

    let level = &texture.levels[0];
    let mut image = Image::new(
        Extent3d {
            width: u32::from(texture.width),
            height: u32::from(texture.height),
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        level.data.clone(),
        format,
        RenderAssetUsages::default(),
    );

    image.sampler = ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor::linear());

    image
}
