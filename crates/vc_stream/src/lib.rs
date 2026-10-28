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
    /// Triangle indices into the mesh's vertex list.
    pub indices: Vec<u32>,
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
            models: HashMap::new(),
            stats: StreamStats::default(),
        }
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

        let bytes = self.fetch(&format!("{}.dff", definition.model))?;
        let geometries =
            vc_dff::decode_clump_with_materials(bytes).map_err(|error| StreamError::Format {
                name: definition.model.clone(),
                reason: error.to_string(),
            })?;

        let resolved: Vec<Vec<Option<Arc<CookedTexture>>>> = geometries
            .iter()
            .map(|geometry| {
                geometry
                    .materials
                    .iter()
                    .map(|material| self.texture(&material.texture, &definition.txd))
                    .collect()
            })
            .collect();

        let model = Arc::new(build_model(&geometries, &resolved, &mut self.stats));

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

    /// Resolves a material's texture name.
    ///
    /// Vice City duplicates shared textures across many dictionaries and points a
    /// model at only one of them, so a miss walks the rest of the archive once and
    /// remembers the answer, including the misses.
    fn texture(&mut self, name: &str, hint: &str) -> Option<Arc<CookedTexture>> {
        let key = name.to_ascii_lowercase();

        if key.is_empty() || self.missing.contains(&key) {
            return None;
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

        None
    }

    fn dictionary(&mut self, txd: &str) -> Option<Arc<HashMap<String, Arc<CookedTexture>>>> {
        if let Some(cached) = self.dictionaries.get(txd) {
            return Some(Arc::clone(cached));
        }

        let bytes = self.fetch(&format!("{txd}.txd")).ok()?;
        let parsed = Dictionary::parse(bytes).ok()?;
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
    resolved: &[Vec<Option<Arc<CookedTexture>>>],
    stats: &mut StreamStats,
) -> CookedModel {
    let mut meshes = Vec::with_capacity(geometries.len());

    for (geometry, textures) in geometries.iter().zip(resolved) {
        if geometry.decoded.triangles.is_empty() || geometry.decoded.vertices.is_empty() {
            continue;
        }

        let vertices: Vec<SceneVertex> = geometry
            .decoded
            .vertices
            .iter()
            .map(|vertex| SceneVertex {
                position: vertex.position,
                normal: vertex.normal,
                uv: vertex.texcoords,
                colour: vertex.colour,
            })
            .collect();

        // Group triangles by material slot so each texture gets its own draw.
        let mut slots: Vec<(Option<Arc<CookedTexture>>, Vec<u32>)> =
            Vec::with_capacity(geometry.materials.len());

        for (material, texture) in geometry.materials.iter().zip(textures) {
            if texture.is_none() && stats.missing.len() < 8 {
                stats.missing.push(material.texture.clone());
            }

            slots.push((texture.clone(), Vec::new()));
        }

        if slots.is_empty() {
            slots.push((None, Vec::new()));
        }

        for triangle in &geometry.decoded.triangles {
            let slot = usize::from(triangle.material).min(slots.len() - 1);
            let group = &mut slots[slot].1;
            group.extend(triangle.indices.map(u32::from));
        }

        let groups = slots
            .into_iter()
            .filter(|(_, indices)| !indices.is_empty())
            .map(|(texture, indices)| MaterialGroup { texture, indices })
            .collect();

        meshes.push(CookedMesh { vertices, groups });
    }

    CookedModel { meshes }
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
        let model = build_model(&geometries, &resolved, &mut StreamStats::default());

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
        let model = build_model(&geometries, &resolved, &mut StreamStats::default());

        assert_eq!(model.meshes[0].groups.len(), 1);
        assert!(model.meshes[0].groups[0].texture.is_none());
    }

    #[test]
    fn clamps_a_material_index_past_the_slot_list() {
        let geometries = vec![geometry(1, &[(7, [0, 1, 2])])];

        let resolved = vec![vec![None; geometries[0].materials.len()]];
        let model = build_model(&geometries, &resolved, &mut StreamStats::default());

        assert_eq!(model.meshes[0].groups.len(), 1);
        assert_eq!(model.meshes[0].groups[0].indices.len(), 3);
    }
}
