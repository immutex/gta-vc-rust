//! Turns decoded Vice City assets into artifacts a GPU can take as-is.
//!
//! The first artifact is textures. Vice City stores block-compressed data directly, so cooking is
//! mostly a matter of splitting the descriptor's payload into mip levels and labelling it: BC1 for
//! the four-bit-per-pixel family and BC3 for the eight-bit one, both of which wgpu accepts as
//! `Bc1RgbaUnorm` and `Bc3RgbaUnorm` with no decode. The 24 paletted rasters are the exception —
//! wgpu has no indexed colour format, so those are expanded to `RGBA8` here.
//!
//! # Examples
//!
//! ```
//! use vc_cook::TextureFormat;
//!
//! // A four-bit-per-pixel family maps straight through.
//! assert_eq!(TextureFormat::for_block_bytes(Some(8)), TextureFormat::Bc1);
//! assert_eq!(TextureFormat::for_block_bytes(Some(16)), TextureFormat::Bc3);
//! ```

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

use thiserror::Error;
use vc_txd::{Family, Texture};

/// Bytes a paletted raster's palette occupies.
pub const PALETTE_BYTES: usize = 1024;

/// Entries in that palette.
pub const PALETTE_ENTRIES: usize = PALETTE_BYTES / 4;

/// GPU texture formats the cook emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TextureFormat {
    /// DXT1 / BC1, four bits per pixel.
    Bc1,
    /// DXT5 / BC3, eight bits per pixel with interpolated alpha.
    Bc3,
    /// Uncompressed, for rasters the GPU cannot take indexed.
    Rgba8,
}

impl TextureFormat {
    /// The format a block-compressed family maps to.
    pub const fn for_block_bytes(block: Option<u32>) -> Self {
        match block {
            Some(8) => Self::Bc1,
            Some(16) => Self::Bc3,
            _ => Self::Rgba8,
        }
    }
}

/// One mip level.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Level {
    /// Level width in pixels.
    pub width: u32,
    /// Level height in pixels.
    pub height: u32,
    /// Texel data, in the texture's format.
    pub data: Vec<u8>,
}

/// A texture ready for upload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CookedTexture {
    /// Texture name.
    pub name: String,
    /// Mask texture name, empty when there is none.
    pub mask: String,
    /// Upload format.
    pub format: TextureFormat,
    /// Level zero width.
    pub width: u16,
    /// Level zero height.
    pub height: u16,
    /// Levels, largest first.
    pub levels: Vec<Level>,
}

impl CookedTexture {
    /// Bytes across every level.
    pub fn byte_len(&self) -> usize {
        self.levels.iter().map(|level| level.data.len()).sum()
    }
}

/// Cooks one texture.
///
/// # Errors
/// Returns [`CookError::Truncated`] when the payload ends inside a level, and
/// [`CookError::ShortPalette`] when a paletted raster is missing its palette.
pub fn cook_texture(texture: &Texture) -> Result<CookedTexture, CookError> {
    let format = TextureFormat::for_block_bytes(texture.block_bytes());
    let levels = match texture.family() {
        Family::Paletted => cook_paletted(texture)?,
        Family::Bc1 | Family::Bc2OrBc3 => cook_block_levels(texture)?,
        Family::Unclassified => return Err(CookError::UnknownFamily),
    };

    Ok(CookedTexture {
        name: texture.name.clone(),
        mask: texture.mask.clone(),
        format,
        width: texture.width,
        height: texture.height,
        levels,
    })
}

fn cook_block_levels(texture: &Texture) -> Result<Vec<Level>, CookError> {
    let mut levels = Vec::with_capacity(usize::from(texture.levels.max(1)));
    let mut cursor = 0;

    for level in 0..texture.levels.max(1) {
        let size = level_bytes(texture, level);

        if level > 0 {
            // Every level after the first carries its own byte length.
            let prefix = texture
                .level_data
                .get(cursor..cursor + 4)
                .ok_or(CookError::Truncated)?;
            cursor += 4;

            let declared = u32::from_le_bytes([prefix[0], prefix[1], prefix[2], prefix[3]]);

            if declared != size {
                return Err(CookError::LevelSize {
                    level,
                    declared,
                    expected: size,
                });
            }
        }

        let end = cursor + size as usize;
        let data = texture
            .level_data
            .get(cursor..end)
            .ok_or(CookError::Truncated)?
            .to_vec();

        levels.push(Level {
            width: u32::from(texture.width) >> level,
            height: u32::from(texture.height) >> level,
            data,
        });

        cursor = end;
    }

    if cursor != texture.level_data.len() {
        return Err(CookError::TrailingBytes {
            consumed: cursor,
            available: texture.level_data.len(),
        });
    }

    Ok(levels)
}

fn level_bytes(texture: &Texture, level: u8) -> u32 {
    let block = texture.block_bytes().unwrap_or(8);
    let divisor = 1u32 << level;
    let width = (u32::from(texture.width) / divisor).max(1);
    let height = (u32::from(texture.height) / divisor).max(1);

    width.div_ceil(4) * height.div_ceil(4) * block
}

fn cook_paletted(texture: &Texture) -> Result<Vec<Level>, CookError> {
    let pixels = usize::from(texture.width) * usize::from(texture.height);
    let expected = pixels + PALETTE_BYTES;

    if texture.level_data.len() != expected {
        return Err(CookError::TrailingBytes {
            consumed: texture.level_data.len(),
            available: expected,
        });
    }

    let (indices, palette) = texture.level_data.split_at(pixels);
    let data = indices
        .iter()
        .flat_map(|index| {
            let at = usize::from(*index) * 4;

            [
                palette[at],
                palette[at + 1],
                palette[at + 2],
                palette[at + 3],
            ]
        })
        .collect();

    Ok(vec![Level {
        width: u32::from(texture.width),
        height: u32::from(texture.height),
        data,
    }])
}

/// Failures while cooking a texture.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CookError {
    /// The payload ended inside a level.
    #[error("texture payload ended inside a level")]
    Truncated,
    /// A level's own length disagreed with the arithmetic.
    #[error("level {level} declares {declared} bytes, arithmetic says {expected}")]
    LevelSize {
        /// Level index.
        level: u8,
        /// Length the file declared.
        declared: u32,
        /// Length the format implies.
        expected: u32,
    },
    /// Bytes were left over, or missing, at the end of the payload.
    #[error("payload holds {available} bytes, the levels account for {consumed}")]
    TrailingBytes {
        /// Bytes the levels used.
        consumed: usize,
        /// Bytes available.
        available: usize,
    },
    /// The raster matched no known family.
    #[error("texture raster matched no known family")]
    UnknownFamily,
}

#[cfg(test)]
mod tests {
    use super::*;
    use vc_txd::{DESCRIPTOR_SIZE, Texture};

    fn texture(width: u16, height: u16, level_bytes: u32, levels: u8, data: Vec<u8>) -> Texture {
        Texture {
            name: "test".to_owned(),
            mask: String::new(),
            platform_id: 8,
            raster_format: 0x0200,
            width,
            height,
            declared_level_bytes: level_bytes,
            struct_len: DESCRIPTOR_SIZE as u32 + data.len() as u32,
            levels,
            level_data: data,
        }
    }

    #[test]
    fn passes_a_single_bc1_level_through() {
        let texture = texture(64, 64, 2048, 1, vec![0xAB; 2048]);

        let cooked = cook_texture(&texture).unwrap();

        assert_eq!(cooked.format, TextureFormat::Bc1);
        assert_eq!(cooked.levels.len(), 1);
        assert_eq!(cooked.byte_len(), 2048);
        assert_eq!(cooked.levels[0].data[0], 0xAB);
    }

    #[test]
    fn labels_eight_bits_per_pixel_as_bc3() {
        let texture = texture(64, 64, 4096, 1, vec![0; 4096]);

        let cooked = cook_texture(&texture).unwrap();

        assert_eq!(cooked.format, TextureFormat::Bc3);
    }

    #[test]
    fn walks_a_mip_chain_with_its_length_prefixes() {
        // 64x64 BC1, two levels: 2048 bytes, then 4-byte length and 512 bytes.
        let mut data = vec![0x11; 2048];
        data.extend(512u32.to_le_bytes());
        data.extend(vec![0x22; 512]);
        let texture = texture(64, 64, 2048, 2, data);

        let cooked = cook_texture(&texture).unwrap();

        assert_eq!(cooked.levels.len(), 2);
        assert_eq!(cooked.levels[1].width, 32);
        assert_eq!(cooked.levels[1].data.len(), 512);
        assert_eq!(cooked.levels[1].data[0], 0x22);
    }

    #[test]
    fn rejects_a_level_whose_length_prefix_disagrees() {
        let mut data = vec![0x11; 2048];
        data.extend(999u32.to_le_bytes());
        data.extend(vec![0x22; 512]);
        let texture = texture(64, 64, 2048, 2, data);

        let result = cook_texture(&texture);

        assert!(matches!(result, Err(CookError::LevelSize { .. })));
    }

    #[test]
    fn expands_a_paletted_raster_to_rgba8() {
        let mut data = vec![0u8, 1, 1, 1]; // four indices
        data.extend(vec![0u8; PALETTE_BYTES]);
        // Palette entry 1 is red.
        data[4 + 4] = 255;
        data[4 + 7] = 255;
        let texture = texture(2, 2, 0, 1, data);

        let cooked = cook_texture(&texture).unwrap();

        assert_eq!(cooked.format, TextureFormat::Rgba8);
        assert_eq!(cooked.levels[0].data.len(), 16);
        assert_eq!(&cooked.levels[0].data[4..8], &[255, 0, 0, 255]);
    }
}
