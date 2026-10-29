//! On-demand cooking for the runtime.
//!
//! Two caches make streaming affordable. Texture dictionaries are parsed once and their cooked
//! textures are shared through `Arc`, because a dictionary is megabytes and dozens of models use it.
//! Models are cooked once and shared the same way, keyed by name, because the map places the same
//! model hundreds of times.
//!
//! Cooking a model groups its triangles by material slot, so each group can be drawn with its own
//! texture — Vice City models routinely use several.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use thiserror::Error;
use vc_cook::{CookedTexture, SceneVertex, cook_texture};
use vc_dff::ClumpGeometry;
use vc_transport::ImageArchive;
use vc_txd::Dictionary;
use vc_world::ObjectDefinition;

/// Triangles sharing one texture.
#[derive(Debug, Clone)]
pub struct MaterialGroup {
    /// Cooked texture, shared with the dictionary cache.
    pub texture: Option<Arc<CookedTexture>>,
    /// Material colour, multiplied into the surface.
    pub colour: [f32; 4],
    /// Triangle indices into the mesh's vertex list.
    pub indices: Vec<u32>,
}

/// One material slot while triangles are being sorted into groups.
struct Slot {
    texture: Option<Arc<CookedTexture>>,
    colour: [f32; 4],
    indices: Vec<u32>,
}

/// A geometry ready to upload.
#[derive(Debug, Clone)]
pub struct CookedMesh {
    /// Vertices.
    pub vertices: Vec<SceneVertex>,
    /// Triangles grouped by the texture they use.
    pub groups: Vec<MaterialGroup>,
}

/// One model, cooked.
#[derive(Debug, Clone, Default)]
pub struct CookedModel {
    /// Geometries, in clump order.
    pub meshes: Vec<CookedMesh>,
}

impl CookedModel {
    /// Triangles across every geometry.
    pub fn triangle_count(&self) -> usize {
        self.meshes
            .iter()
            .map(|mesh| {
                mesh.groups
                    .iter()
                    .map(|group| group.indices.len() / 3)
                    .sum::<usize>()
            })
            .sum()
    }
}

/// Counters for the loading screen and for tests.
#[derive(Debug, Clone, Default)]
pub struct StreamStats {
    /// Models cooked.
    pub models: u64,
    /// Dictionaries parsed.
    pub dictionaries: u64,
    /// Textures cooked.
    pub textures: u64,
    /// Models whose DFF or TXD could not be read.
    pub failures: u64,
    /// Material texture names that no dictionary matched, for diagnosis.
    pub missing: Vec<String>,
    /// Groups whose texture name matched nothing, across every model cooked.
    pub unresolved_groups: u64,
    /// Groups that name no texture at all, across every model cooked.
    pub untextured_groups: u64,
    /// A few keys from the first dictionary parsed, for diagnosis.
    pub keys: Vec<String>,
}

/// Cooks models and shares whatever can be shared.
/// Dictionary that holds the textures models borrow without owning them.
pub const SHARED_TXD: &str = "generic";

/// Cooks models and shares dictionaries, textures and models between them.
pub struct Streamer {
    archive: ImageArchive,
    dictionaries: HashMap<String, Arc<HashMap<String, Arc<CookedTexture>>>>,
    global: HashMap<String, Arc<CookedTexture>>,
    missing: HashSet<String>,
    broken: HashSet<String>,
    models: HashMap<String, Arc<CookedModel>>,
    stats: StreamStats,
}

impl Streamer {
    /// Opens the game's archive.
    ///
    /// # Errors
    /// Returns [`StreamError::Archive`] when the install cannot be opened.
    pub fn open(archive: ImageArchive) -> Self {
        Self {
            archive,
            dictionaries: HashMap::new(),
            global: HashMap::new(),
            missing: HashSet::new(),
            broken: HashSet::new(),
            models: HashMap::new(),
            stats: StreamStats::default(),
        }
    }

    /// The archive this streamer reads, for callers that need other assets.
    pub const fn archive(&self) -> &ImageArchive {
        &self.archive
    }

    /// Counters describing what has been cooked so far.
    pub fn stats(&self) -> StreamStats {
        self.stats.clone()
    }

    /// Cooks a model, or returns the cached one.
    ///
    /// # Errors
    /// Returns [`StreamError::Missing`] when the model is not in the archive and
    /// [`StreamError::Format`] when it does not parse.
    pub fn model(
        &mut self,
        definition: &ObjectDefinition,
    ) -> Result<Arc<CookedModel>, StreamError> {
        if let Some(cached) = self.models.get(&definition.model) {
            return Ok(Arc::clone(cached));
        }

        let bytes = match self.fetch(&format!("{}.dff", definition.model)) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.stats.failures += 1;

                return Err(error);
            }
        };
        let clump = match vc_dff::decode_clump_geometry(bytes) {
            Ok(geometries) => geometries,
            Err(error) => {
                self.stats.failures += 1;

                return Err(StreamError::Format {
                    name: definition.model.clone(),
                    reason: error.to_string(),
                });
            }
        };

        let transforms = clump.geometry_transforms();
        let resolved: Vec<Vec<Option<Arc<CookedTexture>>>> = clump
            .geometries
            .iter()
            .map(|geometry| {
                geometry
                    .materials
                    .iter()
                    .map(|material| self.resolve(&material.texture, &definition.txd))
                    .collect()
            })
            .collect();

        let model = Arc::new(build_model(
            &clump.geometries,
            &transforms,
            &resolved,
            &mut self.stats,
        ));

        self.stats.models += 1;
        let _ = self
            .models
            .insert(definition.model.clone(), Arc::clone(&model));

        Ok(model)
    }

    /// Parses every dictionary once and indexes every texture name.
    ///
    /// Without this the first model that references a shared texture pays for the
    /// whole scan, which is a multi-second stall in the middle of streaming. Doing
    /// it up front turns that into one pause before the window opens.
    pub fn warm_textures(&mut self) -> usize {
        let names: Vec<String> = self
            .archive
            .entries()
            .iter()
            .filter(|entry| entry.has_extension("txd"))
            .map(|entry| entry.name().trim_end_matches(".txd").to_owned())
            .collect();

        for txd in names {
            let Some(dictionary) = self.dictionary(&txd) else {
                continue;
            };

            for (name, texture) in dictionary.iter() {
                let _ = self
                    .global
                    .entry(name.clone())
                    .or_insert_with(|| Arc::clone(texture));
            }
        }

        self.global.len()
    }

    /// Fetches one texture by dictionary and name, for callers outside model cooking.
    pub fn texture(&mut self, txd: &str, name: &str) -> Option<Arc<CookedTexture>> {
        let key = name.to_ascii_lowercase();

        // A dictionary that does not parse must not hide a texture the index already
        // knows about, so the hint is tried first and then simply skipped.
        if let Some(dictionary) = self.dictionary(txd)
            && let Some(found) = dictionary.get(&key)
        {
            return Some(Arc::clone(found));
        }

        self.global.get(&key).cloned()
    }

    /// Resolves a material's texture name.
    ///
    /// Vice City duplicates shared textures across many dictionaries and points a
    /// model at only one of them, so a miss walks the rest of the archive once and
    /// remembers the answer, including the misses.
    fn resolve(&mut self, name: &str, hint: &str) -> Option<Arc<CookedTexture>> {
        let key = name.to_ascii_lowercase();

        if key.is_empty() {
            self.stats.untextured_groups += 1;

            return None;
        }

        if self.missing.contains(&key) {
            self.stats.unresolved_groups += 1;

            return None;
        }

        // The model's own dictionary wins: the same texture name exists at different
        // resolutions in different dictionaries.
        if let Some(dictionary) = self.dictionary(hint)
            && let Some(found) = dictionary.get(&key)
        {
            return Some(Arc::clone(found));
        }

        if let Some(found) = self.global.get(&key) {
            return Some(Arc::clone(found));
        }

        let mut candidates: Vec<String> = vec![hint.to_owned(), SHARED_TXD.to_owned()];
        for entry in self.archive.entries() {
            if entry.has_extension("txd") {
                candidates.push(entry.name().trim_end_matches(".txd").to_owned());
            }
        }

        for txd in candidates {
            let Some(dictionary) = self.dictionary(&txd) else {
                continue;
            };
            let Some(found) = dictionary.get(&key) else {
                continue;
            };

            let _ = self.global.insert(key, Arc::clone(found));

            return Some(Arc::clone(found));
        }

        let _ = self.missing.insert(key);
        self.stats.unresolved_groups += 1;

        None
    }

    fn dictionary(&mut self, txd: &str) -> Option<Arc<HashMap<String, Arc<CookedTexture>>>> {
        if let Some(cached) = self.dictionaries.get(txd) {
            return Some(Arc::clone(cached));
        }

        if self.broken.contains(txd) {
            return None;
        }

        let Some(bytes) = self.fetch(&format!("{txd}.txd")).ok() else {
            let _ = self.broken.insert(txd.to_owned());

            return None;
        };
        let Ok(parsed) = Dictionary::parse(bytes) else {
            let _ = self.broken.insert(txd.to_owned());

            return None;
        };
        let mut by_name = HashMap::with_capacity(parsed.textures.len());

        for texture in &parsed.textures {
            if let Ok(cooked) = cook_texture(texture) {
                self.stats.textures += 1;
                // Keyed lowercase: DFF material names and TXD entries disagree on case
                // often enough in shipped data that an exact match loses textures.
                let key = texture.name.to_ascii_lowercase();

                if self.stats.keys.len() < 8 {
                    self.stats.keys.push(key.clone());
                }

                let _ = by_name.insert(key, Arc::new(cooked));
            }
        }

        self.stats.dictionaries += 1;
        let shared = Arc::new(by_name);
        let _ = self
            .dictionaries
            .insert(txd.to_owned(), Arc::clone(&shared));

        Some(shared)
    }

    fn fetch(&self, name: &str) -> Result<&[u8], StreamError> {
        let indices = self.archive.indices_of(name);
        let Some(&index) = indices.first() else {
            return Err(StreamError::Missing {
                name: name.to_owned(),
            });
        };

        self.archive
            .data(index)
            .map_err(|error| StreamError::Archive {
                reason: error.to_string(),
            })
    }
}

fn build_model(
    geometries: &[ClumpGeometry],
    transforms: &[([[f32; 3]; 3], [f32; 3])],
    resolved: &[Vec<Option<Arc<CookedTexture>>>],
    stats: &mut StreamStats,
) -> CookedModel {
    let mut meshes = Vec::with_capacity(geometries.len());

    for (index, (geometry, textures)) in geometries.iter().zip(resolved).enumerate() {
        if geometry.decoded.triangles.is_empty() || geometry.decoded.vertices.is_empty() {
            continue;
        }

        // A geometry's vertices are in its own frame's space, so the frame transform is
        // applied here and baked in. Without it every part of a multi-part model sits
        // at the model origin instead of where the hierarchy puts it.
        let (rotation, position) = transforms
            .get(index)
            .copied()
            .unwrap_or((vc_dff::identity(), [0.0; 3]));

        let computed = missing_normals(&geometry.decoded);
        let vertices: Vec<SceneVertex> = geometry
            .decoded
            .vertices
            .iter()
            .enumerate()
            .map(|(index, vertex)| {
                let moved = vc_dff::add(vc_dff::rotate(rotation, vertex.position), position);
                let normal = computed
                    .as_ref()
                    .and_then(|normals| normals.get(index).copied())
                    .unwrap_or(vertex.normal);

                SceneVertex {
                    position: moved,
                    normal: vc_dff::rotate(rotation, normal),
                    uv: vertex.texcoords,
                    colour: vertex.colour,
                }
            })
            .collect();

        // Group triangles by material slot so each texture gets its own draw.
        let mut slots: Vec<Slot> = Vec::with_capacity(geometry.materials.len());

        for (material, texture) in geometry.materials.iter().zip(textures) {
            if texture.is_none() && stats.missing.len() < 8 {
                stats.missing.push(material.texture.clone());
            }

            slots.push(Slot {
                texture: texture.clone(),
                colour: material.colour,
                indices: Vec::new(),
            });
        }

        if slots.is_empty() {
            slots.push(Slot {
                texture: None,
                colour: [1.0; 4],
                indices: Vec::new(),
            });
        }

        for triangle in &geometry.decoded.triangles {
            let slot = usize::from(triangle.material).min(slots.len() - 1);
            let group = &mut slots[slot].indices;
            group.extend(triangle.indices.map(u32::from));
        }

        let groups = slots
            .into_iter()
            .filter(|slot| !slot.indices.is_empty())
            .map(|slot| MaterialGroup {
                texture: slot.texture,
                colour: slot.colour,
                indices: slot.indices,
            })
            .collect();

        meshes.push(CookedMesh { vertices, groups });
    }

    CookedModel { meshes }
}

/// Face normals for a geometry that shipped without them.
///
/// The original lit most of the map with baked vertex colour, so some models have no
/// normals at all. A physical renderer would shade those black, so they are derived
/// from the triangles and averaged across shared vertices.
fn missing_normals(decoded: &vc_dff::DecodedGeometry) -> Option<Vec<[f32; 3]>> {
    let has_normals = decoded.vertices.iter().all(|vertex| {
        vertex.normal[0].abs() > f32::EPSILON
            || vertex.normal[1].abs() > f32::EPSILON
            || vertex.normal[2].abs() > f32::EPSILON
    });

    if has_normals {
        return None;
    }

    let mut normals = vec![[0.0f32; 3]; decoded.vertices.len()];
    let mut fallback = vec![None; decoded.vertices.len()];

    for triangle in &decoded.triangles {
        let point = |index: u16| {
            decoded
                .vertices
                .get(usize::from(index))
                .map_or([0.0; 3], |vertex| vertex.position)
        };
        let [first, second, third] = triangle.indices.map(point);
        let edge = |left: [f32; 3], right: [f32; 3]| {
            [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
        };
        let (along, across) = (edge(second, first), edge(third, first));
        let face = [
            along[1].mul_add(across[2], -(along[2] * across[1])),
            along[2].mul_add(across[0], -(along[0] * across[2])),
            along[0].mul_add(across[1], -(along[1] * across[0])),
        ];

        for index in triangle.indices {
            if let Some(slot) = normals.get_mut(usize::from(index)) {
                *slot = vc_dff::add(*slot, face);
            }
            if let Some(slot) = fallback.get_mut(usize::from(index)) {
                slot.get_or_insert(face);
            }
        }
    }

    for (index, normal) in normals.iter_mut().enumerate() {
        let length = normal[0]
            .mul_add(
                normal[0],
                normal[1].mul_add(normal[1], normal[2] * normal[2]),
            )
            .sqrt();

        // Mirrored card pairs - the way Vice City builds foliage - have opposite
        // winding, so their face normals cancel exactly. Keeping one of the two is
        // what makes a lit leaf card visible from both sides.
        if length > f32::EPSILON {
            *normal = [normal[0] / length, normal[1] / length, normal[2] / length];
        } else if let Some(face) = fallback[index] {
            let length = face[0]
                .mul_add(face[0], face[1].mul_add(face[1], face[2] * face[2]))
                .sqrt();
            if length > f32::EPSILON {
                *normal = [face[0] / length, face[1] / length, face[2] / length];
            }
        }
    }

    Some(normals)
}

/// Every collision model in the install, by name.
#[derive(Debug, Clone, Default)]
pub struct Colliders {
    by_name: HashMap<String, vc_physics::Collider>,
}

impl Colliders {
    /// Reads every `.col` file in the archive.
    ///
    /// `peds.col` is known not to parse (PLAN 4.10), so a file that fails is skipped
    /// rather than treated as fatal — its models simply have no collision yet.
    pub fn load(archive: &ImageArchive) -> Self {
        let mut by_name = HashMap::new();

        for entry in archive.entries() {
            if !entry.has_extension("col") {
                continue;
            }

            let Some(&index) = archive.indices_of(entry.name()).first() else {
                continue;
            };
            let Ok(bytes) = archive.data(index) else {
                continue;
            };
            let (collision, _leftover) = vc_col::Collision::parse_lenient(bytes);

            for record in collision.records {
                let _ = by_name.insert(
                    record.name.to_ascii_lowercase(),
                    vc_physics::Collider {
                        vertices: record.vertex_list,
                        faces: record.face_list,
                        spheres: record.sphere_list,
                        boxes: record.box_list,
                    },
                );
            }
        }

        Self { by_name }
    }

    /// The collision model for a model name.
    pub fn get(&self, model: &str) -> Option<&vc_physics::Collider> {
        self.by_name.get(&model.to_ascii_lowercase())
    }

    /// How many models have collision.
    pub fn len(&self) -> usize {
        self.by_name.len()
    }

    /// Whether no model has collision.
    pub fn is_empty(&self) -> bool {
        self.by_name.is_empty()
    }
}

/// Failures while streaming.
#[derive(Debug, Error)]
pub enum StreamError {
    /// The archive could not be opened or read.
    #[error("archive problem: {reason}")]
    Archive {
        /// What went wrong.
        reason: String,
    },
    /// An entry the map asked for is not in the archive.
    #[error("`{name}` is not in the archive")]
    Missing {
        /// Entry name.
        name: String,
    },
    /// An entry did not parse.
    #[error("`{name}` did not parse: {reason}")]
    Format {
        /// Entry name.
        name: String,
        /// What went wrong.
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use vc_dff::{DecodedGeometry, MaterialRef, Triangle, Vertex};

    fn geometry(materials: usize, triangles: &[(u16, [u16; 3])]) -> ClumpGeometry {
        ClumpGeometry {
            decoded: DecodedGeometry {
                flags: 0,
                vertices: vec![Vertex::default(); 3],
                triangles: triangles
                    .iter()
                    .map(|(material, indices)| Triangle {
                        indices: *indices,
                        material: *material,
                    })
                    .collect(),
                has_colours: false,
                has_normals: false,
                texcoord_sets: 0,
            },
            materials: (0..materials)
                .map(|found| MaterialRef {
                    texture: format!("tex{found}"),
                    alpha_mask: String::new(),
                    colour: [1.0; 4],
                })
                .collect(),
        }
    }

    #[test]
    fn groups_triangles_by_material_slot() {
        let geometries = vec![geometry(
            2,
            &[(0, [0, 1, 2]), (1, [2, 1, 0]), (0, [0, 0, 1])],
        )];

        let resolved = vec![vec![None; geometries[0].materials.len()]];
        let identity = vec![(vc_dff::identity(), [0.0; 3]); geometries.len()];
        let model = build_model(
            &geometries,
            &identity,
            &resolved,
            &mut StreamStats::default(),
        );

        assert_eq!(model.meshes.len(), 1);
        assert_eq!(model.meshes[0].groups.len(), 2);
        assert_eq!(model.meshes[0].groups[0].indices.len(), 6);
        assert_eq!(model.meshes[0].groups[1].indices.len(), 3);
        assert_eq!(model.triangle_count(), 3);
    }

    #[test]
    fn falls_back_to_one_group_when_a_geometry_has_no_materials() {
        let geometries = vec![geometry(0, &[(0, [0, 1, 2])])];

        let resolved = vec![vec![None; geometries[0].materials.len()]];
        let identity = vec![(vc_dff::identity(), [0.0; 3]); geometries.len()];
        let model = build_model(
            &geometries,
            &identity,
            &resolved,
            &mut StreamStats::default(),
        );

        assert_eq!(model.meshes[0].groups.len(), 1);
        assert!(model.meshes[0].groups[0].texture.is_none());
    }

    #[test]
    fn clamps_a_material_index_past_the_slot_list() {
        let geometries = vec![geometry(1, &[(7, [0, 1, 2])])];

        let resolved = vec![vec![None; geometries[0].materials.len()]];
        let identity = vec![(vc_dff::identity(), [0.0; 3]); geometries.len()];
        let model = build_model(
            &geometries,
            &identity,
            &resolved,
            &mut StreamStats::default(),
        );

        assert_eq!(model.meshes[0].groups.len(), 1);
        assert_eq!(model.meshes[0].groups[0].indices.len(), 3);
    }
}
