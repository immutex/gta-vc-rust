//! Mip chains for textures that ship with only one level.
//!
//! Vice City's dictionary stores a single level for most textures; sampling those
//! with a mip filter and no chain aliases badly once the whole map is in view. The
//! chain is generated here, in linear light, and re-encoded in the texture's own
//! format so video memory and bandwidth stay compressed.

use std::sync::OnceLock;

use crate::block::{self, Texel};
use crate::{CookedTexture, Level};

/// Builds a full chain from level zero, keeping that level's original bytes.
pub fn generate(texture: &CookedTexture) -> Vec<Level> {
    let Some(level) = texture.levels.first() else {
        return Vec::new();
    };

    let mut levels = vec![level.clone()];
    let mut current = block::decode(texture.format, &level.data, level.width, level.height);
    let (mut width, mut height) = (level.width, level.height);

    while width > 1 || height > 1 {
        (current, width, height) = halve(&current, width, height);
        levels.push(Level {
            width,
            height,
            data: block::encode(texture.format, &current, width, height),
        });
    }

    levels
}

fn halve(source: &[Texel], width: u32, height: u32) -> (Vec<Texel>, u32, u32) {
    let next_width = (width / 2).max(1);
    let next_height = (height / 2).max(1);
    let mut out = vec![[0, 0, 0, 0]; (next_width * next_height) as usize];
    let to_linear = to_linear_table();

    for y in 0..next_height {
        for x in 0..next_width {
            let mut linear = [0.0f32; 3];
            let mut alpha = 0.0f32;

            for dy in 0..2 {
                for dx in 0..2 {
                    let at_x = (x * 2 + dx).min(width - 1);
                    let at_y = (y * 2 + dy).min(height - 1);
                    let texel = source[(at_y * width + at_x) as usize];

                    for channel in 0..3 {
                        linear[channel] += to_linear[usize::from(texel[channel])];
                    }
                    alpha += f32::from(texel[3]);
                }
            }

            out[(y * next_width + x) as usize] = [
                linear_to_srgb(linear[0] / 4.0),
                linear_to_srgb(linear[1] / 4.0),
                linear_to_srgb(linear[2] / 4.0),
                (alpha / 4.0).round() as u8,
            ];
        }
    }

    (out, next_width, next_height)
}

fn to_linear_table() -> &'static [f32; 256] {
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

/// Converts linear light back to sRGB, quantised through a table because this runs
/// once per texel per level and `powf` dominates the cost.
fn linear_to_srgb(value: f32) -> u8 {
    const STEPS: usize = 4096;
    static TABLE: OnceLock<[u8; STEPS + 1]> = OnceLock::new();

    let table = TABLE.get_or_init(|| {
        let mut table = [0u8; STEPS + 1];
        for (index, slot) in table.iter_mut().enumerate() {
            let linear = index as f32 / STEPS as f32;
            let encoded = if linear <= 0.003_130_8 {
                linear * 12.92
            } else {
                1.055f32.mul_add(linear.powf(1.0 / 2.4), -0.055)
            };
            *slot = encoded.mul_add(255.0, 0.5) as u8;
        }

        table
    });

    let index = (value.clamp(0.0, 1.0) * STEPS as f32).round() as usize;

    table[index.min(STEPS)]
}

/// The format-appropriate level count for a texture that starts at `width` by `height`.
pub fn level_count(width: u32, height: u32) -> u32 {
    width.max(height).max(1).ilog2() + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TextureFormat;

    #[test]
    fn counts_the_levels_down_to_one_texel() {
        assert_eq!(level_count(64, 64), 7);
        assert_eq!(level_count(32, 16), 6);
        assert_eq!(level_count(1, 1), 1);
    }

    #[test]
    fn halves_in_the_right_place_not_transposed() {
        // A black square in the top-left quarter. If the filter mixed rows and columns
        // the resulting texel would land in the wrong corner, which on a road texture
        // reads as the pattern running sideways.
        let mut data = vec![255u8; 4 * 4 * 4];
        for y in 0..2 {
            for x in 0..2 {
                let at = (y * 4 + x) * 4;
                data[at..at + 3].fill(0);
            }
        }

        let texture = CookedTexture {
            name: "corner".to_owned(),
            mask: String::new(),
            format: TextureFormat::Rgba8,
            width: 4,
            height: 4,
            levels: vec![Level {
                width: 4,
                height: 4,
                data,
            }],
            alpha: crate::AlphaClass::Opaque,
        };

        let chain = generate(&texture);
        let half = &chain[1];

        assert_eq!((half.width, half.height), (2, 2));

        // Level one is 2x2: its top-left texel averages the black quarter, its top-right
        // averages white, and the bottom row is white.
        assert!(half.data[0] < 40, "top-left should be near black");
        assert!(half.data[4] > 210, "top-right should be near white");
        assert!(half.data[8] > 210, "bottom-left should be near white");
        assert!(half.data[12] > 210, "bottom-right should be near white");
    }

    #[test]
    fn bc1_halves_in_the_right_place_too() {
        let texels: Vec<crate::block::Texel> = (0..4 * 4)
            .map(|index| {
                let (x, y) = (index % 4, index / 4);

                if x < 2 && y < 2 {
                    [0, 0, 0, 255]
                } else {
                    [255, 255, 255, 255]
                }
            })
            .collect();

        let texture = CookedTexture {
            name: "corner".to_owned(),
            mask: String::new(),
            format: TextureFormat::Bc1,
            width: 4,
            height: 4,
            levels: vec![Level {
                width: 4,
                height: 4,
                data: crate::block::encode(TextureFormat::Bc1, &texels, 4, 4),
            }],
            alpha: crate::AlphaClass::Opaque,
        };

        let chain = generate(&texture);
        let half = &chain[1];
        let decoded = crate::block::decode(TextureFormat::Bc1, &half.data, 2, 2);

        assert_eq!((half.width, half.height), (2, 2));
        assert!(decoded[0][0] < 60, "top-left should be dark");
        assert!(decoded[1][0] > 180, "top-right should be light");
        assert!(decoded[2][0] > 180, "bottom-left should be light");
        assert!(decoded[3][0] > 180, "bottom-right should be light");
    }

    #[test]
    fn halves_into_a_full_chain() {
        let texture = CookedTexture {
            name: "test".to_owned(),
            mask: String::new(),
            format: TextureFormat::Rgba8,
            width: 4,
            height: 4,
            levels: vec![Level {
                width: 4,
                height: 4,
                data: vec![255; 4 * 4 * 4],
            }],
            alpha: crate::AlphaClass::Opaque,
        };

        let chain = generate(&texture);

        assert_eq!(chain.len(), 3);
        assert_eq!((chain[1].width, chain[1].height), (2, 2));
        assert_eq!((chain[2].width, chain[2].height), (1, 1));
        assert!(
            chain
                .iter()
                .all(|level| level.data.iter().any(|byte| *byte > 200))
        );
    }

    #[test]
    fn averages_in_linear_light_not_gamma_space() {
        assert_eq!(linear_to_srgb(0.0), 0);
        assert!(linear_to_srgb(0.5) > 180);
        assert!(linear_to_srgb(1.0) >= 254);
    }
}
