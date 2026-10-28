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
use bevy::render::render_resource::encase::ShaderType;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, RenderPipelineDescriptor, SpecializedMeshPipelineError,
    TextureDimension, TextureFormat,
};
use bevy::shader::ShaderRef;
use vc_cook::CookedTexture;
use vc_stream::CookedModel;

/// Path of the material shader, relative to the asset root.
pub const SHADER_PATH: &str = "shaders/vc_material.wesl";

/// Values the fragment shader reads for a material.
#[derive(Clone, Debug, ShaderType)]
pub struct VcUniforms {
    /// Colour multiplier.
    pub tint: Vec4,
    /// Above zero, texels with alpha below half are discarded.
    pub cutout: f32,
    /// Lowest shade a baked surface may have.
    pub floor: f32,
    /// Light that reaches surfaces regardless of their baked shade.
    pub ambient: Vec4,
    /// Multiplier for the baked shade, which already contains the original's sun.
    pub sun: Vec4,
}

/// How the world is lit: an ambient floor plus a gain on the baked prelit shade.
#[derive(Clone, Copy, Debug)]
pub struct Lighting {
    /// Ambient colour, added to the prelit shade.
    pub ambient: Vec4,
    /// Gain applied to the prelit shade.
    pub sun: Vec4,
    /// Lowest shade a baked surface may have.
    ///
    /// Vice City's roads and under-passes are baked black because the original lit them
    /// with dynamic lights at runtime - 24% of the world's prelit vertices are near
    /// black. Without those lights, a floor stands in for what they would have added.
    pub floor: f32,
}

impl Default for Lighting {
    fn default() -> Self {
        Self {
            ambient: Vec4::ONE,
            sun: Vec4::ONE,
            floor: 0.45,
        }
    }
}

/// A `RenderWare`-flavoured material: one texture, one tint.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct VcMaterial {
    /// Diffuse texture.
    #[texture(0)]
    #[sampler(1)]
    pub diffuse: Option<Handle<Image>>,
    /// Shader uniforms.
    #[uniform(2)]
    pub uniforms: VcUniforms,
    /// Whether to discard cut-out texels. Kept beside the uniforms so `alpha_mode`
    /// and the shader agree without allocating.
    pub cutout: bool,
}

impl Default for VcMaterial {
    fn default() -> Self {
        Self {
            diffuse: None,
            uniforms: VcUniforms {
                tint: Vec4::ONE,
                cutout: 0.0,
                floor: 0.0,
                ambient: Vec4::ZERO,
                sun: Vec4::ONE,
            },
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
        // Vice City's signage, railings and foliage are single quads seen from both
        // sides; culling the back face makes them vanish as the camera swings round.
        descriptor.primitive.cull_mode = None;

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

/// Keeps one GPU texture per cooked texture, however many models ask for it.
#[derive(Default, Resource)]
pub struct TextureCache {
    /// Keyed by allocation address, holding the `Arc` so no address can be recycled
    /// while it is a key.
    uploaded: HashMap<usize, (Arc<CookedTexture>, Handle<Image>)>,
    /// One-pixel grey, for geometry whose material names no texture.
    fallback: Option<Handle<Image>>,
}

impl TextureCache {
    /// Uploads a texture once and hands back the shared handle.
    pub fn handle(
        &mut self,
        texture: &Arc<CookedTexture>,
        images: &mut Assets<Image>,
    ) -> Handle<Image> {
        let key = Arc::as_ptr(texture) as usize;

        if let Some((_, handle)) = self.uploaded.get(&key) {
            return handle.clone();
        }

        let handle = images.add(upload_texture(texture));
        let _ = self
            .uploaded
            .insert(key, (Arc::clone(texture), handle.clone()));

        handle
    }

    /// A flat grey image for materials that name no texture.
    ///
    /// The original draws those with the material colour; without it, grey is far closer
    /// than the white that Bevy's own fallback produces, and closer than dropping the
    /// surface, which left holes in the beach.
    pub fn flat_grey(&mut self, images: &mut Assets<Image>) -> Handle<Image> {
        if let Some(handle) = &self.fallback {
            return handle.clone();
        }

        let mut image = Image::new(
            Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            vec![128, 128, 128, 255],
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        );
        image.sampler = ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor::linear());

        let handle = images.add(image);
        self.fallback = Some(handle.clone());

        handle
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
    lighting: Lighting,
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

        for group in &mesh.groups {
            // Each material group gets its own mesh, so it is compiled down to only the
            // vertices it indexes. Uploading a full copy of the vertex buffer per group
            // multiplied memory by the number of materials a geometry uses.
            let mut remap = vec![u32::MAX; mesh.vertices.len()];
            let mut indices = Vec::with_capacity(group.indices.len());
            let mut positions = Vec::with_capacity(group.indices.len());
            let mut normals = Vec::with_capacity(group.indices.len());
            let mut uvs = Vec::with_capacity(group.indices.len());
            let mut colours = Vec::with_capacity(group.indices.len());

            for index in &group.indices {
                let Some(source) = mesh.vertices.get(*index as usize) else {
                    continue;
                };
                let slot = remap[*index as usize];

                if slot == u32::MAX {
                    let next = positions.len() as u32;
                    remap[*index as usize] = next;
                    positions.push(source.position);
                    normals.push(source.normal);
                    uvs.push(source.uv[0]);
                    colours.push([
                        prelit_channel(source.colour[0]),
                        prelit_channel(source.colour[1]),
                        prelit_channel(source.colour[2]),
                        f32::from(source.colour[3]) / 255.0,
                    ]);
                    indices.push(next);
                } else {
                    indices.push(slot);
                }
            }

            if indices.is_empty() {
                continue;
            }

            let mut built = Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::RENDER_WORLD,
            );
            built.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
            built.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
            built.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
            built.insert_attribute(Mesh::ATTRIBUTE_COLOR, colours);
            built.insert_indices(Indices::U32(indices));

            // A group with no resolved texture draws with flat grey rather than Bevy's
            // white default, which read as a hole in the world; white patches on the
            // beach were exactly this.
            let diffuse = match group.texture.as_ref() {
                Some(texture) => textures.handle(texture, images),
                None => textures.flat_grey(images),
            };
            let cutout = group
                .texture
                .as_ref()
                .is_some_and(|texture| texture.has_alpha);

            parts.push(ModelPart {
                mesh: meshes.add(built),
                material: materials.add(VcMaterial {
                    diffuse: Some(diffuse),
                    uniforms: VcUniforms {
                        tint: Vec4::ONE,
                        cutout: if cutout { 1.0 } else { 0.0 },
                        floor: lighting.floor,
                        ambient: lighting.ambient,
                        sun: lighting.sun,
                    },
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
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_descriptor.mip_level_count = texture.levels.len().max(1) as u32;

    // Vice City's texture coordinates tile well past 0..1, so the sampler must repeat
    // rather than clamp, or every tiled surface smears its edge texel.
    image.sampler = ImageSampler::Descriptor(bevy::image::ImageSamplerDescriptor {
        address_mode_u: bevy::image::ImageAddressMode::Repeat,
        address_mode_v: bevy::image::ImageAddressMode::Repeat,
        address_mode_w: bevy::image::ImageAddressMode::Repeat,
        ..bevy::image::ImageSamplerDescriptor::linear()
    });

    image
}
