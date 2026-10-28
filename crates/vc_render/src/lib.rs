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
    /// Whether to discard cut-out texels.
    pub cutout: bool,
}

impl Default for VcMaterial {
    fn default() -> Self {
        Self {
            diffuse: None,
            tint: LinearRgba::WHITE,
            cutout: false,
        }
    }
}

impl Material for VcMaterial {
    // The default mesh vertex shader, because our shader only has a fragment entry.
    fn fragment_shader() -> ShaderRef {
        SHADER_PATH.into()
    }

    // Only textures that actually carry transparency are masked; masking everything
    // punched holes wherever a texture's alpha happened to be zero.
    fn alpha_mode(&self) -> AlphaMode {
        if self.cutout {
            AlphaMode::Mask(0.5)
        } else {
            AlphaMode::Opaque
        }
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // Location 5 for colour is Bevy's own convention, which the default mesh
        // vertex shader reads; any other number would silently miss it.
        let vertex_layout = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_NORMAL.at_shader_location(1),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(2),
            Mesh::ATTRIBUTE_COLOR.at_shader_location(5),
        ])?;
        descriptor.vertex.buffers = vec![vertex_layout];

        Ok(())
    }
}

/// Registers the material with the app.
pub struct VcRenderPlugin;

impl Plugin for VcRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<VcMaterial>::default())
            .init_resource::<TextureCache>();
    }
}

/// Converts a world position from the game's axes to Bevy's.
///
/// The data is Z-up with X east and Y north; Bevy is Y-up with -Z forward. Mapping
/// `(x, y, z)` to `(x, z, -y)` is a rotation, so handedness is preserved and the
/// city is not mirrored.
pub fn to_bevy_position(position: [f32; 3]) -> Vec3 {
    Vec3::new(position[0], position[2], -position[1])
}

/// Converts a rotation the same way: the axis moves, the angle does not.
pub fn to_bevy_rotation(rotation: [f32; 4]) -> Quat {
    Quat::from_xyzw(rotation[0], rotation[2], -rotation[1], rotation[3])
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
        let colours: Vec<[f32; 4]> = mesh
            .vertices
            .iter()
            .map(|vertex| {
                [
                    prelit_channel(vertex.colour[0]),
                    prelit_channel(vertex.colour[1]),
                    prelit_channel(vertex.colour[2]),
                    f32::from(vertex.colour[3]) / 255.0,
                ]
            })
            .collect();

        for group in &mesh.groups {
            let mut built = Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::default(),
            );
            built.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions.clone());
            built.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals.clone());
            built.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs.clone());
            built.insert_attribute(Mesh::ATTRIBUTE_COLOR, colours.clone());
            built.insert_indices(Indices::U32(group.indices.clone()));

            let diffuse = group
                .texture
                .as_ref()
                .map(|texture| textures.handle(texture, images));
            let cutout = group
                .texture
                .as_ref()
                .is_some_and(|texture| texture.has_alpha);

            parts.push(ModelPart {
                mesh: meshes.add(built),
                material: materials.add(VcMaterial {
                    diffuse,
                    tint: LinearRgba::WHITE,
                    cutout,
                }),
            });
        }
    }

    parts
}

/// Prelit colours are multiplied in the space they were authored in.
///
/// Vice City's fixed-function pipeline multiplied texture by vertex colour without
/// leaving gamma space, so the bytes are the shading. Linearising them here darkened
/// the whole city by roughly a square: a mid-grey 0.5 becomes 0.21.
fn prelit_channel(channel: u8) -> f32 {
    f32::from(channel) / 255.0
}

fn upload_texture(texture: &CookedTexture) -> Image {
    let format = match texture.format {
        vc_cook::TextureFormat::Bc1 => TextureFormat::Bc1RgbaUnorm,
        vc_cook::TextureFormat::Bc2 => TextureFormat::Bc2RgbaUnorm,
        vc_cook::TextureFormat::Rgba8 => TextureFormat::Rgba8UnormSrgb,
    };

    // Every level is uploaded, not just the largest. Sampling a city's worth of
    // block-compressed texture with a single level shimmers badly at distance, which
    // reads as glitching rather than as aliasing.
    let mut data = Vec::new();
    for level in &texture.levels {
        data.extend_from_slice(&level.data);
    }

    let mut image = Image::new(
        Extent3d {
            width: u32::from(texture.width),
            height: u32::from(texture.height),
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        format,
        RenderAssetUsages::default(),
    );
    image.texture_descriptor.mip_level_count = texture.levels.len().max(1) as u32;

    image.sampler = ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor::linear());

    image
}
