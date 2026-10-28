//! Block-compressed texel access.
//!
//! Vice City ships DXT1 and DXT3 blocks, so the runtime only needs those two
//! families and their uncompressed fallback. Decoding is exact, as the GPU would do
//! it; encoding is a bounding-box quantiser, which is what makes a generated mip
//! chain possible without a compressor dependency.

use crate::TextureFormat;

/// One texel, sRGB-encoded with straight alpha.
pub type Texel = [u8; 4];

/// Decodes a level into texels, clamping partial edge blocks.
pub fn decode(format: TextureFormat, data: &[u8], width: u32, height: u32) -> Vec<Texel> {
    let mut texels = vec![[0, 0, 0, 0]; (width * height) as usize];

    match format {
        TextureFormat::Bc1 => decode_colour(data, width, height, &mut texels, false),
        TextureFormat::Bc2 => {
            decode_colour(
                data.get(8..).unwrap_or_default(),
                width,
                height,
                &mut texels,
                true,
            );
            decode_alpha(data, width, height, &mut texels);
        }
        TextureFormat::Rgba8 => {
            for (texel, bytes) in texels.iter_mut().zip(data.as_chunks::<4>().0) {
                *texel = [bytes[0], bytes[1], bytes[2], bytes[3]];
            }
        }
    }

    texels
}

/// Encodes texels into a level, in the texture's own format.
pub fn encode(format: TextureFormat, texels: &[Texel], width: u32, height: u32) -> Vec<u8> {
    match format {
        TextureFormat::Bc1 => encode_colour(texels, width, height, true),
        TextureFormat::Bc2 => {
            let mut out = encode_alpha(texels, width, height);
            out.extend_from_slice(&encode_colour(texels, width, height, false));

            out
        }
        TextureFormat::Rgba8 => texels.iter().flat_map(|texel| *texel).collect(),
    }
}

fn decode_colour(data: &[u8], width: u32, height: u32, texels: &mut [Texel], always_four: bool) {
    let blocks_x = width.div_ceil(4);
    let blocks_y = height.div_ceil(4);
    let mut cursor = 0;

    for block_y in 0..blocks_y {
        for block_x in 0..blocks_x {
            let Some(block) = data.get(cursor..cursor + 8) else {
                return;
            };
            cursor += 8;

            let first = u16::from_le_bytes([block[0], block[1]]);
            let second = u16::from_le_bytes([block[2], block[3]]);
            let indices = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
            let palette = colour_palette(first, second, always_four);

            for y in 0..4 {
                for x in 0..4 {
                    let index = ((indices >> (2 * (y * 4 + x))) & 0b11) as usize;
                    let at_x = block_x * 4 + x;
                    let at_y = block_y * 4 + y;

                    if at_x < width && at_y < height {
                        texels[(at_y * width + at_x) as usize] = palette[index];
                    }
                }
            }
        }
    }
}

fn decode_alpha(data: &[u8], width: u32, height: u32, texels: &mut [Texel]) {
    let blocks_x = width.div_ceil(4);
    let blocks_y = height.div_ceil(4);
    let mut cursor = 0;

    for block_y in 0..blocks_y {
        for block_x in 0..blocks_x {
            let Some(block) = data.get(cursor..cursor + 8) else {
                return;
            };
            cursor += 8;

            let nibbles = u64::from_le_bytes([
                block[0], block[1], block[2], block[3], block[4], block[5], block[6], block[7],
            ]);

            for y in 0..4 {
                for x in 0..4 {
                    let nibble = (nibbles >> (4 * (y * 4 + x))) & 0xF;
                    let at_x = block_x * 4 + x;
                    let at_y = block_y * 4 + y;

                    if at_x < width && at_y < height {
                        texels[(at_y * width + at_x) as usize][3] = (nibble * 17) as u8;
                    }
                }
            }
        }
    }
}

fn colour_palette(first: u16, second: u16, always_four: bool) -> [Texel; 4] {
    let a = from_565(first);
    let b = from_565(second);

    if always_four || first > second {
        [a, b, blend(a, b, 2, 1, 3), blend(a, b, 1, 2, 3)]
    } else {
        [a, b, blend(a, b, 1, 1, 2), [0, 0, 0, 0]]
    }
}

fn from_565(value: u16) -> Texel {
    let r = u32::from((value >> 11) & 0x1F);
    let g = u32::from((value >> 5) & 0x3F);
    let b = u32::from(value & 0x1F);

    [
        ((r * 255 + 15) / 31) as u8,
        ((g * 255 + 31) / 63) as u8,
        ((b * 255 + 15) / 31) as u8,
        255,
    ]
}

fn to_565(colour: [u8; 3]) -> u16 {
    let r = (u16::from(colour[0]) * 31 + 127) / 255;
    let g = (u16::from(colour[1]) * 63 + 127) / 255;
    let b = (u16::from(colour[2]) * 31 + 127) / 255;

    (r << 11) | (g << 5) | b
}

fn blend(a: Texel, b: Texel, weight_a: u16, weight_b: u16, total: u16) -> Texel {
    let mix = |left: u8, right: u8| {
        ((u16::from(left) * weight_a + u16::from(right) * weight_b) / total) as u8
    };

    [mix(a[0], b[0]), mix(a[1], b[1]), mix(a[2], b[2]), 255]
}

fn encode_colour(texels: &[Texel], width: u32, height: u32, punch_through: bool) -> Vec<u8> {
    let blocks_x = width.div_ceil(4);
    let blocks_y = height.div_ceil(4);
    let mut out = Vec::with_capacity((blocks_x * blocks_y * 8) as usize);

    for block_y in 0..blocks_y {
        for block_x in 0..blocks_x {
            let mut colours = [[0u8; 3]; 16];
            let mut transparent = [false; 16];

            for y in 0..4 {
                for x in 0..4 {
                    let at_x = (block_x * 4 + x).min(width.saturating_sub(1));
                    let at_y = (block_y * 4 + y).min(height.saturating_sub(1));
                    let texel = texels[(at_y * width + at_x) as usize];
                    let slot = (y * 4 + x) as usize;

                    colours[slot] = [texel[0], texel[1], texel[2]];
                    transparent[slot] = punch_through && texel[3] < 128;
                }
            }

            let has_punch = transparent.iter().any(|clear| *clear);
            let (low, high) = bounds(&colours, &transparent);
            let (first, second) = endpoints(low, high, has_punch);
            let palette = colour_palette(first, second, !has_punch);
            let opaque_limit = if has_punch { 2 } else { 3 };

            let mut indices = 0u32;

            for (slot, colour) in colours.iter().enumerate() {
                let index = if transparent[slot] {
                    3
                } else {
                    nearest(palette, *colour, opaque_limit)
                };
                indices |= u32::from(index as u8) << (2 * slot);
            }

            out.extend_from_slice(&first.to_le_bytes());
            out.extend_from_slice(&second.to_le_bytes());
            out.extend_from_slice(&indices.to_le_bytes());
        }
    }

    out
}

fn bounds(colours: &[[u8; 3]; 16], transparent: &[bool; 16]) -> ([u8; 3], [u8; 3]) {
    let mut low = [255u8; 3];
    let mut high = [0u8; 3];
    let mut seen = false;

    for (colour, clear) in colours.iter().zip(transparent) {
        if *clear {
            continue;
        }
        seen = true;

        for channel in 0..3 {
            low[channel] = low[channel].min(colour[channel]);
            high[channel] = high[channel].max(colour[channel]);
        }
    }

    if seen { (low, high) } else { ([0; 3], [0; 3]) }
}

fn endpoints(low: [u8; 3], high: [u8; 3], punch_through: bool) -> (u16, u16) {
    let mut inset_low = low;
    let mut inset_high = high;

    for channel in 0..3 {
        let step = (u16::from(high[channel]) - u16::from(low[channel])) / 16;
        let step = step as u8;
        inset_low[channel] = low[channel].saturating_add(step);
        inset_high[channel] = high[channel].saturating_sub(step);
    }

    let mut first = to_565(inset_high);
    let mut second = to_565(inset_low);

    // The decoder picks its palette mode from the endpoint order, so the order is part
    // of the format: four colours need the first endpoint to compare greater, three
    // colours plus transparency need the opposite.
    if punch_through {
        if first > second {
            std::mem::swap(&mut first, &mut second);
        }
    } else if first <= second {
        if first == 0 {
            first = 1;
        } else {
            second = first - 1;
        }
    }

    (first, second)
}

fn nearest(palette: [Texel; 4], colour: [u8; 3], limit: usize) -> usize {
    (0..=limit)
        .min_by_key(|index| {
            let candidate = palette[*index];
            (0..3)
                .map(|channel| {
                    let delta = i32::from(colour[channel]) - i32::from(candidate[channel]);
                    delta * delta
                })
                .sum::<i32>()
        })
        .unwrap_or(0)
}

fn encode_alpha(texels: &[Texel], width: u32, height: u32) -> Vec<u8> {
    let blocks_x = width.div_ceil(4);
    let blocks_y = height.div_ceil(4);
    let mut out = Vec::with_capacity((blocks_x * blocks_y * 8) as usize);

    for block_y in 0..blocks_y {
        for block_x in 0..blocks_x {
            let mut nibbles = 0u64;

            for y in 0..4 {
                for x in 0..4 {
                    let at_x = (block_x * 4 + x).min(width.saturating_sub(1));
                    let at_y = (block_y * 4 + y).min(height.saturating_sub(1));
                    let alpha = texels[(at_y * width + at_x) as usize][3] >> 4;
                    nibbles |= u64::from(alpha) << (4 * (y * 4 + x));
                }
            }

            out.extend_from_slice(&nibbles.to_le_bytes());
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_a_solid_bc1_block() {
        let texels = [[40u8, 90, 200, 255]; 16];
        let encoded = encode(TextureFormat::Bc1, &texels, 4, 4);
        let decoded = decode(TextureFormat::Bc1, &encoded, 4, 4);

        for texel in decoded {
            assert!(texel[0].abs_diff(40) <= 4, "{texel:?}");
            assert!(texel[1].abs_diff(90) <= 4, "{texel:?}");
            assert!(texel[2].abs_diff(200) <= 4, "{texel:?}");
            assert_eq!(texel[3], 255);
        }
    }

    #[test]
    fn keeps_punch_through_alpha() {
        let mut texels = [[255u8, 0, 0, 255]; 16];
        texels[5] = [0, 0, 0, 0];
        texels[6] = [255, 255, 255, 255];

        let encoded = encode(TextureFormat::Bc1, &texels, 4, 4);
        let decoded = decode(TextureFormat::Bc1, &encoded, 4, 4);

        assert_eq!(decoded[5][3], 0);
        assert_eq!(decoded[0][3], 255);
    }

    #[test]
    fn round_trips_bc2_alpha() {
        let mut texels = [[10u8, 20, 30, 255]; 16];
        texels[3][3] = 0x40;

        let encoded = encode(TextureFormat::Bc2, &texels, 4, 4);
        let decoded = decode(TextureFormat::Bc2, &encoded, 4, 4);

        assert!(decoded[3][3].abs_diff(0x40) <= 17, "{}", decoded[3][3]);
        assert_eq!(decoded[0][3], 255);
    }

    #[test]
    fn pads_blocks_past_an_odd_edge() {
        let texels = [[1u8, 2, 3, 255]; 7 * 5];

        for format in [TextureFormat::Bc1, TextureFormat::Bc2] {
            let encoded = encode(format, &texels, 7, 5);
            let decoded = decode(format, &encoded, 7, 5);

            assert_eq!(decoded.len(), 35);
            assert_eq!(decoded[34][3], 255);
        }
    }
}
