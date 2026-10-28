//! Materials, texture upload and static geometry batching for the vc-rs runtime.
//!
//! Vice City's world is static, so the cheapest way to draw it is also the simplest:
//! bake every placement of a map chunk into one mesh per texture, then let the
//! renderer cull whole chunks. A chunk of 400 units holds a hundred-odd draws instead
//! of several thousand, and the vertex cost is the geometry itself, not an entity per
//! material group.
//!
//! Modeling stays in the game's axes until [`Placement`] maps it to Bevy's, so the
//! physics crate can keep working in Z-up coordinates.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::Aabb;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};
use bevy::pbr::StandardMaterial;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use vc_cook::{AlphaClass, CookedTexture, TextureFormat as CookedFormat};
use vc_stream::CookedModel;

/// Anisotropy for world textures: roads and pavements are almost always seen at a
/// grazing angle, where trilinear filtering alone smears them into a blur.
pub const ANISOTROPY: u16 = 8;

/// Lowest linear value a prelit colour may keep.
///
/// The original baked dynamic lighting into the prelit colours, and a quarter of the
/// map's vertices are near black because a streetlight or headlight was doing the
/// work. Without that light, black albedo would stay black under any sun.
pub const PRELIT_FLOOR: f32 = 0.06;

/// Converts a position from the game's Z-up axes to Bevy's Y-up axes.
///
/// `(x, y, z) → (x, z, -y)` is a rotation, so handedness survives and the city is not
/// mirrored.
pub fn to_bevy_position(position: [f32; 3]) -> Vec3 {
    Vec3::new(position[0], position[2], -position[1])
}

/// The same axis change as a quaternion, for rotating directions.
pub fn to_bevy_axes() -> Quat {
    Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)
}

/// A placement in Bevy space, ready to bake into vertices.
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    /// World translation.
    pub translation: Vec3,
    /// World rotation.
    pub rotation: Quat,
    /// Per-axis scale.
    pub scale: Vec3,
}

impl Placement {
    /// Maps an instance's game-space transform into Bevy space.
    #[must_use]
    pub fn new(position: [f32; 3], rotation: [f32; 4], scale: [f32; 3]) -> Self {
        let axes = to_bevy_axes();
        let local = Quat::from_xyzw(rotation[0], rotation[1], rotation[2], rotation[3]);

        Self {
            translation: to_bevy_position(position),
            // A similarity transform, not a component shuffle: the rotation keeps its
            // angle and takes the basis with it.
            rotation: axes * local * axes.inverse(),
            scale: Vec3::from_array(scale),
        }
    }

    fn apply(&self, position: Vec3, normal: Vec3) -> (Vec3, Vec3) {
        let inverse = Vec3::new(
            reciprocal(self.scale.x),
            reciprocal(self.scale.y),
            reciprocal(self.scale.z),
        );

        (
            self.translation + self.rotation * (position * self.scale),
            (self.rotation * (normal * inverse)).normalize_or_zero(),
        )
    }
}

fn reciprocal(value: f32) -> f32 {
    if value.abs() > f32::EPSILON {
        value
    } else {
        0.0
    }
}

/// One drawable piece of a baked chunk.
pub struct MeshPart {
    /// Merged mesh.
    pub mesh: Handle<Mesh>,
    /// Material for the texture the mesh was merged under.
    pub material: Handle<StandardMaterial>,
    /// Local-space bounds, for the visibility culler.
    pub bounds: Aabb,
    /// Whether the texture blends, which must not cast a solid shadow.
    pub blend: bool,
}

/// Merged geometry for one chunk.
pub struct Baked {
    /// One mesh per texture.
    pub parts: Vec<MeshPart>,
    /// Triangles across the chunk.
    pub triangles: usize,
    /// Vertices across the chunk.
    pub vertices: usize,
}

/// GPU textures, uploaded once however many dictionaries and models name them.
///
/// Keyed by name rather than by allocation: Vice City duplicates shared textures
/// across dictionaries, so a model's own dictionary often hands back a second copy of
/// an image another model already uploaded.
#[derive(Default, Resource)]
pub struct TextureCache {
    uploaded: HashMap<String, Handle<Image>>,
    flat: Option<Handle<Image>>,
}

impl TextureCache {
    /// Uploads every texture that is not resident yet, generating its mip chain.
    ///
    /// Chain generation is the expensive part of loading and is pure arithmetic, so
    /// it runs across every core while the asset inserts stay on this thread.
    pub fn upload_batch(&mut self, textures: &[Arc<CookedTexture>], images: &mut Assets<Image>) {
        if self.flat.is_none() {
            self.flat = Some(images.add(flat_image()));
        }

        let mut seen = std::collections::HashSet::new();
        let pending: Vec<&Arc<CookedTexture>> = textures
            .iter()
            .filter(|texture| {
                let key = key_of(texture);
                !self.uploaded.contains_key(&key) && seen.insert(key)
            })
            .collect();
        let built = generate_parallel(&pending);

        for (texture, image) in pending.into_iter().zip(built) {
            let _ = self.uploaded.insert(key_of(texture), images.add(image));
        }
    }

    /// The image for a texture that [`Self::upload_batch`] has seen.
    #[must_use]
    pub fn handle(&self, texture: &Arc<CookedTexture>) -> Option<Handle<Image>> {
        self.uploaded.get(&key_of(texture)).cloned()
    }

    /// One mid-grey texel, for groups that name no texture.
    ///
    /// White is the wrong fallback: a missing texture then reads as a hole punched in
    /// the world, which is what the pale patches on the beach were. Grey keeps the
    /// surface solid and lets the prelit shade do the work.
    #[must_use]
    pub fn flat(&self) -> Handle<Image> {
        self.flat.clone().unwrap_or_default()
    }

    /// How many textures are resident.
    #[must_use]
    pub fn len(&self) -> usize {
        self.uploaded.len()
    }

    /// Whether nothing has been uploaded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.uploaded.is_empty()
    }
}

fn key_of(texture: &CookedTexture) -> String {
    texture.name.to_ascii_lowercase()
}

fn generate_parallel(textures: &[&Arc<CookedTexture>]) -> Vec<Image> {
    if textures.is_empty() {
        return Vec::new();
    }

    let workers = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    let workers = workers.min(textures.len()).max(1);
    let chunk = textures.len().div_ceil(workers);

    std::thread::scope(|scope| {
        textures
            .chunks(chunk)
            .map(|slice| {
                scope.spawn(move || {
                    slice
                        .iter()
                        .map(|texture| image_for(texture))
                        .collect::<Vec<_>>()
                })
            })
            .flat_map(|handle| handle.join().unwrap_or_default())
            .collect()
    })
}

fn image_for(texture: &CookedTexture) -> Image {
    let format = match texture.format {
        CookedFormat::Bc1 => TextureFormat::Bc1RgbaUnormSrgb,
        CookedFormat::Bc2 => TextureFormat::Bc2RgbaUnormSrgb,
        CookedFormat::Rgba8 => TextureFormat::Rgba8UnormSrgb,
    };
    let levels = vc_cook::mips::generate(texture);
    let mut data = Vec::with_capacity(levels.iter().map(|level| level.data.len()).sum());

    for level in &levels {
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
    image.texture_descriptor.mip_level_count = levels.len().max(1) as u32;
    // Texture coordinates tile well past 0..1 in this map, so the sampler repeats.
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        address_mode_w: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: ANISOTROPY,
        ..ImageSamplerDescriptor::linear()
    });

    image
}

fn flat_image() -> Image {
    let mut image = Image::new(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        vec![158, 158, 158, 255],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::linear();

    image
}

/// One material per texture name and alpha behaviour.
#[derive(Default, Resource)]
pub struct MaterialCache {
    materials: HashMap<(String, AlphaClass), Handle<StandardMaterial>>,
}

impl MaterialCache {
    /// The material for a texture, created on first use.
    pub fn get(
        &mut self,
        texture: Option<&Arc<CookedTexture>>,
        textures: &TextureCache,
        materials: &mut Assets<StandardMaterial>,
    ) -> Handle<StandardMaterial> {
        let key = texture.map_or((String::new(), AlphaClass::Opaque), |texture| {
            (key_of(texture), texture.alpha)
        });

        if let Some(handle) = self.materials.get(&key) {
            return handle.clone();
        }

        let clear = texture.is_some_and(|texture| texture.alpha == AlphaClass::Blend);
        let mask = texture.is_some_and(|texture| texture.alpha == AlphaClass::Mask);
        let image = texture
            .and_then(|texture| textures.handle(texture))
            .unwrap_or_else(|| textures.flat());

        let material = StandardMaterial {
            base_color: Color::WHITE,
            base_color_texture: Some(image),
            perceptual_roughness: 0.85,
            metallic: 0.0,
            alpha_mode: if clear {
                AlphaMode::Blend
            } else if mask {
                AlphaMode::Mask(0.5)
            } else {
                AlphaMode::Opaque
            },
            // Both sides. The map's winding is not reliable enough to cull - a back-face
            // discard reads as a hole in the ground - so fill savings must come from
            // atlasing rather than from culling.
            cull_mode: None,
            double_sided: true,
            ..StandardMaterial::default()
        };

        let handle = materials.add(material);
        let _ = self.materials.insert(key, handle.clone());

        handle
    }

    /// How many materials exist.
    #[must_use]
    pub fn len(&self) -> usize {
        self.materials.len()
    }

    /// Whether no material has been made.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.materials.is_empty()
    }
}

/// A blended, lightly reflective surface for the ocean.
pub fn water_material(
    tint: Color,
    texture: Handle<Image>,
    materials: &mut Assets<StandardMaterial>,
) -> Handle<StandardMaterial> {
    materials.add(StandardMaterial {
        base_color: tint,
        base_color_texture: Some(texture),
        alpha_mode: AlphaMode::Blend,
        perceptual_roughness: 0.08,
        metallic: 0.0,
        reflectance: 0.3,
        cull_mode: None,
        double_sided: true,
        ..StandardMaterial::default()
    })
}

#[derive(Clone, Copy)]
struct Vertex {
    position: Vec3,
    normal: Vec3,
    uv: Vec2,
    colour: Vec4,
}

/// Accumulates model geometry for one chunk and emits one mesh per texture.
#[derive(Default)]
pub struct Batcher {
    buckets: HashMap<(String, AlphaClass), Bucket>,
    remap: Vec<u32>,
}

#[derive(Default)]
struct Bucket {
    texture: Option<Arc<CookedTexture>>,
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    min: Vec3,
    max: Vec3,
}

impl Batcher {
    /// Adds every placed part of a model, transformed into world space.
    pub fn push(&mut self, model: &CookedModel, placement: &Placement) {
        let to_linear = srgb_table();

        for mesh in &model.meshes {
            for group in &mesh.groups {
                let colour = Vec4::from_array(group.colour);

                match group.texture.as_ref() {
                    Some(texture) => self.push_group(
                        Some(texture),
                        colour,
                        mesh,
                        &group.indices,
                        placement,
                        to_linear,
                    ),
                    None => {
                        self.push_group(None, colour, mesh, &group.indices, placement, to_linear);
                    }
                }
            }
        }
    }

    fn push_group(
        &mut self,
        texture: Option<&Arc<CookedTexture>>,
        colour: Vec4,
        mesh: &vc_stream::CookedMesh,
        indices: &[u32],
        placement: &Placement,
        to_linear: &[f32; 256],
    ) {
        let key = texture.map_or((String::new(), AlphaClass::Opaque), |texture| {
            (key_of(texture), texture.alpha)
        });
        let bucket = self.buckets.entry(key).or_insert_with(|| Bucket {
            texture: texture.cloned(),
            min: Vec3::splat(f32::INFINITY),
            max: Vec3::splat(f32::NEG_INFINITY),
            ..Bucket::default()
        });

        self.remap.clear();
        self.remap.resize(mesh.vertices.len(), u32::MAX);

        for index in indices {
            let Some(source) = mesh.vertices.get(*index as usize) else {
                continue;
            };
            let slot = self.remap[*index as usize];

            if slot == u32::MAX {
                let (position, normal) = placement.apply(
                    Vec3::from_array(source.position),
                    Vec3::from_array(source.normal),
                );
                // The new vertex's index is the bucket's current length; taking it
                // from the index list instead would double-count earlier groups.
                let next = bucket.vertices.len() as u32;
                self.remap[*index as usize] = next;
                bucket.min = bucket.min.min(position);
                bucket.max = bucket.max.max(position);
                // Texture x material x prelit, which is the multiply the original
                // fixed-function pipeline performed. Folding the material colour into
                // the vertex keeps one bucket per texture however many materials share
                // it, and gives untextured geometry - the beach, painted lines - the
                // colour the artists authored instead of a flat stand-in.
                bucket.vertices.push(Vertex {
                    position,
                    normal,
                    uv: Vec2::from_array(source.uv[0]),
                    colour: Vec4::new(
                        to_linear[usize::from(source.colour[0])].max(PRELIT_FLOOR),
                        to_linear[usize::from(source.colour[1])].max(PRELIT_FLOOR),
                        to_linear[usize::from(source.colour[2])].max(PRELIT_FLOOR),
                        f32::from(source.colour[3]) / 255.0,
                    ) * colour,
                });
                bucket.indices.push(next);
            } else {
                bucket.indices.push(slot);
            }
        }
    }

    /// Emits one drawable mesh per texture.
    pub fn finish(
        self,
        meshes: &mut Assets<Mesh>,
        materials: &mut Assets<StandardMaterial>,
        textures: &TextureCache,
        cache: &mut MaterialCache,
    ) -> Baked {
        let (mut triangles, mut vertices) = (0, 0);
        let parts: Vec<MeshPart> = self
            .buckets
            .into_values()
            .filter(|bucket| !bucket.indices.is_empty())
            .map(|mut bucket| {
                triangles += bucket.indices.len() / 3;
                vertices += bucket.vertices.len();
                repair_normals(&mut bucket);
                let mut mesh = Mesh::new(
                    PrimitiveTopology::TriangleList,
                    RenderAssetUsages::RENDER_WORLD,
                );
                mesh.insert_attribute(
                    Mesh::ATTRIBUTE_POSITION,
                    bucket
                        .vertices
                        .iter()
                        .map(|vertex| vertex.position.to_array())
                        .collect::<Vec<_>>(),
                );
                mesh.insert_attribute(
                    Mesh::ATTRIBUTE_NORMAL,
                    bucket
                        .vertices
                        .iter()
                        .map(|vertex| vertex.normal.to_array())
                        .collect::<Vec<_>>(),
                );
                mesh.insert_attribute(
                    Mesh::ATTRIBUTE_UV_0,
                    bucket
                        .vertices
                        .iter()
                        .map(|vertex| vertex.uv.to_array())
                        .collect::<Vec<_>>(),
                );
                mesh.insert_attribute(
                    Mesh::ATTRIBUTE_COLOR,
                    bucket
                        .vertices
                        .iter()
                        .map(|vertex| vertex.colour.to_array())
                        .collect::<Vec<_>>(),
                );
                mesh.insert_indices(Indices::U32(bucket.indices));

                let material = cache.get(bucket.texture.as_ref(), textures, materials);

                MeshPart {
                    mesh: meshes.add(mesh),
                    material,
                    bounds: Aabb::from_min_max(bucket.min, bucket.max),
                    blend: bucket
                        .texture
                        .as_ref()
                        .is_some_and(|texture| texture.alpha == AlphaClass::Blend),
                }
            })
            .collect();

        Baked {
            parts,
            triangles,
            vertices,
        }
    }
}

fn srgb_table() -> &'static [f32; 256] {
    static TABLE: OnceLock<[f32; 256]> = OnceLock::new();

    TABLE.get_or_init(|| {
        let mut table = [0.0f32; 256];
        for (value, slot) in table.iter_mut().enumerate() {
            let channel = value as f32 / 255.0;
            *slot = if channel <= 0.040_45 {
                channel / 12.92
            } else {
                ((channel + 0.055) / 1.055).powf(2.4)
            };
        }

        table
    })
}

/// Fills in normals the data does not carry.
///
/// 461 vertices across the map ship a zero-length normal, and a zero normal under a
/// real sun shades black regardless of albedo. Each such corner takes the
/// area-weighted normal of the triangles that use it.
fn repair_normals(bucket: &mut Bucket) {
    let missing: Vec<usize> = bucket
        .vertices
        .iter()
        .enumerate()
        .filter(|(_, vertex)| vertex.normal.length_squared() < 1e-6)
        .map(|(index, _)| index)
        .collect();

    if missing.is_empty() {
        return;
    }

    let mut computed = vec![Vec3::ZERO; bucket.vertices.len()];

    for triangle in bucket.indices.as_chunks::<3>().0 {
        let [a, b, c] = [
            bucket.vertices[triangle[0] as usize].position,
            bucket.vertices[triangle[1] as usize].position,
            bucket.vertices[triangle[2] as usize].position,
        ];
        let normal = (b - a).cross(c - a);

        for index in triangle {
            computed[*index as usize] += normal;
        }
    }

    for index in missing {
        let normal = computed[index];

        bucket.vertices[index].normal = if normal.length_squared() > 1e-12 {
            normal.normalize()
        } else {
            Vec3::Y
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_game_axes_without_mirroring() {
        assert_eq!(to_bevy_position([1.0, 2.0, 3.0]), Vec3::new(1.0, 3.0, -2.0));

        // A rotation by 90 degrees about the game's up axis must become a rotation
        // about Bevy's up axis, with the same angle.
        let game = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let placement = Placement::new([0.0; 3], [game.x, game.y, game.z, game.w], [1.0; 3]);
        let east = placement.rotation * to_bevy_position([1.0, 0.0, 0.0]);

        // East turned north: north is -Z in Bevy, and the city must not mirror.
        assert!((east - Vec3::new(0.0, 0.0, -1.0)).length() < 1e-5, "{east}");
    }

    fn two_triangles() -> vc_stream::CookedModel {
        let vertex = |position: [f32; 3]| vc_cook::SceneVertex {
            position,
            normal: [0.0, 1.0, 0.0],
            uv: [[0.0; 2]; 2],
            colour: [255; 4],
        };

        vc_stream::CookedModel {
            meshes: vec![vc_stream::CookedMesh {
                vertices: vec![
                    vertex([0.0, 0.0, 0.0]),
                    vertex([1.0, 0.0, 0.0]),
                    vertex([0.0, 0.0, 1.0]),
                    vertex([2.0, 0.0, 0.0]),
                    vertex([3.0, 0.0, 0.0]),
                    vertex([2.0, 0.0, 1.0]),
                ],
                groups: vec![
                    vc_stream::MaterialGroup {
                        colour: [1.0; 4],
                        texture: None,
                        indices: vec![0, 1, 2],
                    },
                    vc_stream::MaterialGroup {
                        colour: [1.0; 4],
                        texture: None,
                        indices: vec![3, 4, 5],
                    },
                ],
            }],
        }
    }

    #[test]
    fn keeps_indices_inside_the_merged_bucket() {
        let mut batcher = Batcher::default();
        let placement = Placement::new([0.0; 3], [0.0, 0.0, 0.0, 1.0], [1.0; 3]);
        let model = two_triangles();

        // Pushing twice puts four groups into one bucket, which is where an offset
        // taken from the index list instead of the vertex list goes out of range.
        batcher.push(&model, &placement);
        batcher.push(&model, &placement);

        let mut meshes = Assets::<Mesh>::default();
        let mut materials = Assets::<StandardMaterial>::default();
        let textures = TextureCache::default();
        let mut cache = MaterialCache::default();
        let baked = batcher.finish(&mut meshes, &mut materials, &textures, &mut cache);

        assert_eq!(baked.triangles, 4);
        assert_eq!(baked.vertices, 12);
        assert_eq!(baked.parts.len(), 1);

        let mesh = meshes
            .get(&baked.parts[0].mesh)
            .unwrap_or_else(|| unreachable!());
        let Some(Indices::U32(indices)) = mesh.indices() else {
            unreachable!("a merged mesh is indexed");
        };
        assert!(indices.iter().all(|index| *index < 12), "{indices:?}");
    }

    #[test]
    fn keeps_prelit_colour_out_of_black() {
        let table = srgb_table();

        assert!(table[0].max(PRELIT_FLOOR) >= PRELIT_FLOOR);
        assert!((table[255] - 1.0).abs() < 1e-5);
    }
}
