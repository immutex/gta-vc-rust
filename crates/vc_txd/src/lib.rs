//! Reader for Vice City TXD texture dictionaries.
//!
//! A dictionary declares a texture count and then holds one `TextureNative`
//! section per texture. Each of those opens with a struct holding the platform
//! id, the texture and mask names, the raster format, the dimensions and an
//! explicitly declared byte length, followed by the level data itself.
//!
//! PC Vice City stores most textures as a single DXT1 level with no mip chain,
//! so the declared length usually equals `width * height / 2`. Textures that do
//! not fit that shape are reported rather than rejected, because the full raster
//! format set is still being mapped.
//!
//! # Examples
//!
//! ```no_run
//! # use vc_txd::Dictionary;
//! # fn example(bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
//! let dictionary = Dictionary::parse(bytes)?;
//!
//! for texture in &dictionary.textures {
//!     println!("{} {}x{}", texture.name, texture.width, texture.height);
//! }
//! # Ok(())
//! # }
//! ```

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

use rw_stream::{Section, SectionKind, StreamError};
use thiserror::Error;

/// Bytes of the texture descriptor that precede the level data.
pub const DESCRIPTOR_SIZE: usize = 92;

/// Offset of the texture name within the descriptor.
pub const NAME_OFFSET: usize = 8;

/// Width of the name and mask fields.
pub const NAME_WIDTH: usize = 32;

/// Offset of the mask name within the descriptor.
pub const MASK_OFFSET: usize = 40;

/// Offset of the raster format word within the descriptor.
pub const RASTER_FORMAT_OFFSET: usize = 72;

/// Offset of the width and height, as two `u16`s, within the descriptor.
pub const SIZE_OFFSET: usize = 80;

/// Offset of the declared byte length of level zero.
pub const LEVEL_BYTES_OFFSET: usize = 88;

/// One texture inside a dictionary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Texture {
    /// Texture name.
    pub name: String,
    /// Mask texture name.
    pub mask: String,
    /// Platform id, 8 for Direct3D 8 in Vice City.
    pub platform_id: u32,
    /// Raster format word, for example `0x0200` or `0x8200`.
    pub raster_format: u32,
    /// Texture width in pixels.
    pub width: u16,
    /// Texture height in pixels.
    pub height: u16,
    /// Byte length of level zero, declared by the file.
    pub declared_level_bytes: u32,
    /// Length of the descriptor struct's payload.
    pub struct_len: u32,
}

impl Texture {
    /// Bytes a single DXT1 level of this texture occupies.
    pub fn dxt1_level_bytes(&self) -> u32 {
        u32::from(self.width) * u32::from(self.height) / 2
    }

    /// Whether the declared level length matches a single DXT1 level.
    pub fn is_single_dxt1_level(&self) -> bool {
        self.declared_level_bytes == self.dxt1_level_bytes() && self.declared_level_bytes != 0
    }

    /// Whether both dimensions are non-zero powers of two.
    pub const fn is_power_of_two(&self) -> bool {
        self.width.is_power_of_two() && self.height.is_power_of_two()
    }

    /// Bytes of the descriptor struct beyond the descriptor and level zero.
    pub const fn unexplained_tail(&self) -> u32 {
        self.struct_len
            .saturating_sub(DESCRIPTOR_SIZE as u32)
            .saturating_sub(self.declared_level_bytes)
    }
}

/// A parsed texture dictionary.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Dictionary {
    /// Texture count declared by the dictionary struct.
    pub declared_count: u32,
    /// Textures that parsed.
    pub textures: Vec<Texture>,
    /// Textures whose descriptor could not be read.
    pub skipped: u32,
}

impl Dictionary {
    /// Parses the dictionary in `bytes`.
    ///
    /// # Errors
    /// Returns [`TxdError::UnexpectedRoot`] when the file is not a texture
    /// dictionary, [`TxdError::MissingStruct`] when the dictionary has no struct
    /// child, and propagates [`StreamError`] for malformed section trees.
    pub fn parse(bytes: &[u8]) -> Result<Self, TxdError> {
        let root = Section::root(bytes)?;

        if root.kind() != SectionKind::TEXTURE_DICTIONARY {
            return Err(TxdError::UnexpectedRoot { kind: root.kind() });
        }

        let dictionary_struct =
            root.child(SectionKind::STRUCT)?
                .ok_or(TxdError::MissingStruct {
                    parent: SectionKind::TEXTURE_DICTIONARY,
                })?;

        let mut dictionary = Self {
            declared_count: dictionary_struct.u32_at(0)?,
            ..Self::default()
        };

        for texture in root.children_of(SectionKind::TEXTURE_NATIVE)? {
            match read_texture(&texture) {
                Some(texture) => dictionary.textures.push(texture),
                None => dictionary.skipped += 1,
            }
        }

        Ok(dictionary)
    }

    /// Number of textures whose declared level length matches a single DXT1 level.
    pub fn single_dxt1_levels(&self) -> usize {
        self.textures
            .iter()
            .filter(|texture| texture.is_single_dxt1_level())
            .count()
    }

    /// Number of textures with dimensions that are not powers of two.
    pub fn non_power_of_two(&self) -> usize {
        self.textures
            .iter()
            .filter(|texture| !texture.is_power_of_two())
            .count()
    }

    /// Number of textures whose descriptor struct holds bytes beyond the
    /// descriptor and level zero.
    pub fn textures_with_tail(&self) -> usize {
        self.textures
            .iter()
            .filter(|texture| texture.unexplained_tail() != 0)
            .count()
    }
}

fn read_texture(section: &Section<'_>) -> Option<Texture> {
    let descriptor = section.child(SectionKind::STRUCT).ok()??;

    if descriptor.payload().len() < DESCRIPTOR_SIZE {
        return None;
    }

    let (width, height) = descriptor.u16x2_at(SIZE_OFFSET).ok()?;

    Some(Texture {
        name: descriptor.string_at(NAME_OFFSET, NAME_WIDTH)?,
        mask: descriptor.string_at(MASK_OFFSET, NAME_WIDTH)?,
        platform_id: descriptor.u32_at(0).ok()?,
        raster_format: descriptor.u32_at(RASTER_FORMAT_OFFSET).ok()?,
        width,
        height,
        declared_level_bytes: descriptor.u32_at(LEVEL_BYTES_OFFSET).ok()?,
        struct_len: descriptor.size(),
    })
}

/// Failures while reading a texture dictionary.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TxdError {
    /// The file's top level section is not a texture dictionary.
    #[error("expected a texture dictionary, found section kind {kind}")]
    UnexpectedRoot {
        /// The kind that was found.
        kind: SectionKind,
    },
    /// The dictionary has no struct child stating the texture count.
    #[error("texture dictionary has no struct child")]
    MissingStruct {
        /// The parent section kind.
        parent: SectionKind,
    },
    /// The section tree is malformed.
    #[error(transparent)]
    Stream(#[from] StreamError),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn section(kind: SectionKind, payload: &[u8]) -> Vec<u8> {
        let mut bytes = kind.0.to_le_bytes().to_vec();
        bytes.extend((payload.len() as u32).to_le_bytes());
        bytes.extend(0x1003_FFFFu32.to_le_bytes());
        bytes.extend(payload);

        bytes
    }

    fn texture(name: &str, width: u16, height: u16, level_bytes: u32) -> Vec<u8> {
        let mut descriptor = vec![0u8; DESCRIPTOR_SIZE];
        descriptor[0..4].copy_from_slice(&8u32.to_le_bytes());
        descriptor[NAME_OFFSET..NAME_OFFSET + name.len()].copy_from_slice(name.as_bytes());
        descriptor[RASTER_FORMAT_OFFSET..RASTER_FORMAT_OFFSET + 4]
            .copy_from_slice(&0x0200u32.to_le_bytes());
        descriptor[SIZE_OFFSET..SIZE_OFFSET + 2].copy_from_slice(&width.to_le_bytes());
        descriptor[SIZE_OFFSET + 2..SIZE_OFFSET + 4].copy_from_slice(&height.to_le_bytes());
        descriptor[LEVEL_BYTES_OFFSET..LEVEL_BYTES_OFFSET + 4]
            .copy_from_slice(&level_bytes.to_le_bytes());
        descriptor.extend(vec![0u8; level_bytes as usize]);

        section(
            SectionKind::TEXTURE_NATIVE,
            &section(SectionKind::STRUCT, &descriptor),
        )
    }

    fn dictionary(textures: &[Vec<u8>]) -> Vec<u8> {
        let mut body = section(SectionKind::STRUCT, &(textures.len() as u32).to_le_bytes());

        for texture in textures {
            body.extend(texture);
        }

        section(SectionKind::TEXTURE_DICTIONARY, &body)
    }

    #[test]
    fn reads_texture_names_and_dimensions() {
        let bytes = dictionary(&[texture("kb_ivy2_256", 256, 256, 32768)]);

        let parsed = Dictionary::parse(&bytes).unwrap();

        assert_eq!(parsed.textures[0].name, "kb_ivy2_256");
        assert_eq!(parsed.textures[0].width, 256);
    }

    #[test]
    fn reports_the_declared_texture_count() {
        let bytes = dictionary(&[texture("a", 32, 32, 512), texture("b", 64, 64, 2048)]);

        let parsed = Dictionary::parse(&bytes).unwrap();

        assert_eq!(parsed.declared_count, 2);
        assert_eq!(parsed.textures.len(), 2);
    }

    #[test]
    fn recognises_a_single_dxt1_level() {
        let bytes = dictionary(&[texture("step_64HV", 64, 64, 2048)]);

        let parsed = Dictionary::parse(&bytes).unwrap();

        assert!(parsed.textures[0].is_single_dxt1_level());
        assert_eq!(parsed.single_dxt1_levels(), 1);
    }

    #[test]
    fn reports_a_level_length_that_is_not_dxt1_shaped() {
        let bytes = dictionary(&[texture("odd", 64, 64, 4096)]);

        let parsed = Dictionary::parse(&bytes).unwrap();

        assert!(!parsed.textures[0].is_single_dxt1_level());
    }

    #[test]
    fn reports_non_power_of_two_dimensions() {
        let bytes = dictionary(&[texture("nonpot", 48, 64, 1536)]);

        let parsed = Dictionary::parse(&bytes).unwrap();

        assert_eq!(parsed.non_power_of_two(), 1);
    }

    #[test]
    fn rejects_a_file_that_is_not_a_dictionary() {
        let bytes = section(SectionKind::CLUMP, &[]);

        let result = Dictionary::parse(&bytes);

        assert!(matches!(result, Err(TxdError::UnexpectedRoot { .. })));
    }
}
