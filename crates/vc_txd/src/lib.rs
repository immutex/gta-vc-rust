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

/// Offset of the field whose second byte holds the mip level count.
pub const LEVEL_COUNT_OFFSET: usize = 84;

/// Bytes per 4×4 block for DXT1, four bits per pixel.
pub const BC1_BLOCK_BYTES: u32 = 8;

/// Bytes per 4×4 block for DXT3 and DXT5, eight bits per pixel.
pub const BC23_BLOCK_BYTES: u32 = 16;

/// Bytes of a 256-entry `RGBA` palette.
pub const PALETTE_BYTES: u32 = 1024;

/// Bytes a level's own size prefix occupies, for every level after the first.
pub const LEVEL_PREFIX_BYTES: u32 = 4;

/// Which storage family a texture belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Family {
    /// DXT1 / BC1, four bits per pixel.
    Bc1,
    /// DXT3 or DXT5, eight bits per pixel. The two share a byte budget, so the
    /// level sizes cannot tell them apart.
    Bc2OrBc3,
    /// Eight bits per pixel indexed into a 256-entry palette.
    Paletted,
    /// A raster that none of the above accounts for.
    Unclassified,
}

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
    /// Mip levels declared by the descriptor, one when it carries no chain.
    pub levels: u8,
    /// Everything after the descriptor: level zero, then each later level behind
    /// its `u32` byte length, plus a palette for paletted rasters.
    pub level_data: Vec<u8>,
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

    /// Which family the level data puts this texture in.
    pub const fn family(&self) -> Family {
        let pixels = self.pixel_count();

        if pixels != 0 && self.declared_level_bytes == pixels / 2 {
            Family::Bc1
        } else if pixels != 0 && self.declared_level_bytes == pixels {
            Family::Bc2OrBc3
        } else if pixels != 0 && self.level_data_bytes() == pixels + PALETTE_BYTES {
            Family::Paletted
        } else {
            Family::Unclassified
        }
    }

    /// Bytes per 4×4 block, for the two block-compressed families.
    pub const fn block_bytes(&self) -> Option<u32> {
        match self.family() {
            Family::Bc1 => Some(BC1_BLOCK_BYTES),
            Family::Bc2OrBc3 => Some(BC23_BLOCK_BYTES),
            _ => None,
        }
    }

    /// Pixels in level zero.
    pub const fn pixel_count(&self) -> u32 {
        self.width as u32 * self.height as u32
    }

    /// Bytes the whole descriptor should hold after the 92-byte descriptor.
    ///
    /// Level zero follows the descriptor directly; every later level is preceded
    /// by its own `u32` byte length, which is why a chain costs four bytes per
    /// extra level beyond the data itself.
    pub fn chain_bytes(&self) -> Option<u32> {
        if self.family() == Family::Paletted {
            return Some(self.pixel_count() + PALETTE_BYTES);
        }

        let block = self.block_bytes()?;
        let mut total = self.declared_level_bytes;

        for level in 1..self.levels.max(1) {
            total += LEVEL_PREFIX_BYTES + level_bytes(self.width, self.height, level, block);
        }

        Some(total)
    }

    /// Bytes the descriptor struct holds after the descriptor itself.
    pub const fn level_data_bytes(&self) -> u32 {
        self.struct_len.saturating_sub(DESCRIPTOR_SIZE as u32)
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

    /// Textures in the DXT1 family.
    pub fn bc1_count(&self) -> usize {
        self.family_count(Family::Bc1)
    }

    /// Textures in the DXT3 or DXT5 family.
    pub fn bc23_count(&self) -> usize {
        self.family_count(Family::Bc2OrBc3)
    }

    /// Textures stored as paletted rasters.
    pub fn paletted_count(&self) -> usize {
        self.family_count(Family::Paletted)
    }

    /// Textures whose raster is neither block-compressed family.
    pub fn unclassified_count(&self) -> usize {
        self.family_count(Family::Unclassified)
    }

    /// Textures that declare more than one mip level.
    pub fn with_mips(&self) -> usize {
        self.textures
            .iter()
            .filter(|texture| texture.levels > 1)
            .count()
    }

    /// Textures whose declared mip chain accounts for their level data exactly.
    pub fn chains_exact(&self) -> usize {
        self.textures
            .iter()
            .filter(|texture| texture.chain_bytes() == Some(texture.level_data_bytes()))
            .count()
    }

    fn family_count(&self, family: Family) -> usize {
        self.textures
            .iter()
            .filter(|texture| texture.family() == family)
            .count()
    }
}

/// Bytes a mip level occupies, rounded up to whole blocks and never below one.
fn level_bytes(width: u16, height: u16, level: u8, block: u32) -> u32 {
    let divisor = 1u32 << level;
    let width = (u32::from(width) / divisor).max(1);
    let height = (u32::from(height) / divisor).max(1);

    width.div_ceil(4) * height.div_ceil(4) * block
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
        levels: (descriptor.u32_at(LEVEL_COUNT_OFFSET).ok()? >> 8) as u8,
        level_data: descriptor
            .payload()
            .get(DESCRIPTOR_SIZE..)
            .unwrap_or_default()
            .to_vec(),
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
        texture_with_levels(name, width, height, level_bytes, 1, level_bytes)
    }

    /// Builds a texture whose descriptor declares `levels` and carries `data`
    /// bytes of level data.
    fn texture_with_levels(
        name: &str,
        width: u16,
        height: u16,
        level_bytes: u32,
        levels: u8,
        data: u32,
    ) -> Vec<u8> {
        let mut descriptor = vec![0u8; DESCRIPTOR_SIZE];
        descriptor[0..4].copy_from_slice(&8u32.to_le_bytes());
        descriptor[NAME_OFFSET..NAME_OFFSET + name.len()].copy_from_slice(name.as_bytes());
        descriptor[RASTER_FORMAT_OFFSET..RASTER_FORMAT_OFFSET + 4]
            .copy_from_slice(&0x0200u32.to_le_bytes());
        descriptor[SIZE_OFFSET..SIZE_OFFSET + 2].copy_from_slice(&width.to_le_bytes());
        descriptor[SIZE_OFFSET + 2..SIZE_OFFSET + 4].copy_from_slice(&height.to_le_bytes());
        descriptor[LEVEL_BYTES_OFFSET..LEVEL_BYTES_OFFSET + 4]
            .copy_from_slice(&level_bytes.to_le_bytes());
        descriptor[LEVEL_COUNT_OFFSET + 1] = levels;
        descriptor.extend(vec![0u8; data as usize]);

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
        let bytes = dictionary(&[texture("odd", 64, 64, 1234)]);

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

    #[test]
    fn classifies_half_a_byte_per_pixel_as_dxt1() {
        let bytes = dictionary(&[texture("road", 128, 128, 8192)]);

        let parsed = Dictionary::parse(&bytes).unwrap();

        assert_eq!(parsed.textures[0].family(), Family::Bc1);
        assert_eq!(parsed.bc1_count(), 1);
    }

    #[test]
    fn classifies_a_byte_per_pixel_as_dxt3_or_dxt5() {
        let bytes = dictionary(&[texture("window", 128, 128, 16384)]);

        let parsed = Dictionary::parse(&bytes).unwrap();

        assert_eq!(parsed.textures[0].family(), Family::Bc2OrBc3);
        assert_eq!(parsed.bc23_count(), 1);
    }

    #[test]
    fn leaves_an_unrecognised_raster_unclassified() {
        let bytes = dictionary(&[texture("odd", 64, 64, 1234)]);

        let parsed = Dictionary::parse(&bytes).unwrap();

        assert_eq!(parsed.unclassified_count(), 1);
    }

    #[test]
    fn accounts_for_a_single_level_chain() {
        let bytes = dictionary(&[texture("single", 64, 64, 2048)]);

        let parsed = Dictionary::parse(&bytes).unwrap();

        assert_eq!(parsed.chains_exact(), 1);
    }

    #[test]
    fn classifies_a_paletted_raster() {
        let bytes = dictionary(&[texture_with_levels("fence", 64, 64, 0, 1, 4096 + 1024)]);

        let parsed = Dictionary::parse(&bytes).unwrap();

        assert_eq!(parsed.textures[0].family(), Family::Paletted);
        assert_eq!(parsed.chains_exact(), 1);
    }

    #[test]
    fn accounts_for_a_full_mip_chain() {
        // 64x64 DXT1, seven levels: 2048, then 512/128/32/8/8/8 each prefixed
        // by four bytes.
        let bytes = dictionary(&[texture_with_levels("chained", 64, 64, 2048, 7, 2768)]);

        let parsed = Dictionary::parse(&bytes).unwrap();

        assert_eq!(parsed.chains_exact(), 1);
    }

    #[test]
    fn notices_a_mip_chain_that_does_not_add_up() {
        let bytes = dictionary(&[texture_with_levels("short", 64, 64, 2048, 7, 2048)]);

        let parsed = Dictionary::parse(&bytes).unwrap();

        assert_eq!(parsed.chains_exact(), 0);
    }
}
