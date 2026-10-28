//! Sky, sun and fog for one hour of one weather type.
//!
//! `timecyc.dat` holds the colour of every hour but no sun position, because the
//! original computed that in code. The direction below is the same spherical sweep:
//! east at sunrise, highest at noon, west at sunset.

use std::error::Error;
use std::fs;
use std::path::Path;

use bevy::prelude::*;
use vc_timecyc::Timecycle;

use vc_render::to_bevy_position;

/// How far past the original's far clip the fog keeps fading.
///
/// The map is about six thousand units across. Fading at the original two thousand
/// would put the far shore in solid haze, which reads as missing geometry once the
/// whole city is resident.
pub const FOG_REACH: f32 = 2.5;

/// The sky and lighting of one hour.
#[derive(Resource, Clone)]
pub struct Sky {
    /// Weather type drawn, for the console.
    pub weather: String,
    /// Hour of the weather row used.
    pub hour: usize,
    /// Horizon colour, also the clear colour.
    pub clear: Color,
    /// Water tint from the timecyc.
    pub water: Color,
    /// Ambient light colour.
    pub ambient: Color,
    /// Directional light colour.
    pub sun: Color,
    /// Directional light scale, zero at night and one at noon.
    pub power: f32,
    /// Distance where fog starts.
    pub fog_start: f32,
    /// Distance where fog reaches the horizon colour.
    pub fog_end: f32,
}

impl Sky {
    /// Reads the hour's row for the first weather type.
    ///
    /// # Errors
    /// Returns an error when the file cannot be read, does not parse, or does not
    /// hold the requested hour.
    pub fn load(path: &Path, hour: usize) -> Result<Self, Box<dyn Error>> {
        let text = String::from_utf8_lossy(&fs::read(path)?).into_owned();
        let table = Timecycle::parse(&text)?;
        let weather = table
            .weathers
            .first()
            .ok_or("timecyc.dat holds no weather")?;
        let row = weather
            .hours
            .get(hour)
            .ok_or("timecyc.dat is missing that hour")?;
        let power = row.directional[0]
            .max(row.directional[1])
            .max(row.directional[2])
            / 255.0;

        Ok(Self {
            weather: weather.name.clone(),
            hour,
            clear: srgb(row.sky_bottom),
            water: srgb([row.water[0], row.water[1], row.water[2]]),
            ambient: srgb(row.ambient),
            sun: srgb(row.directional),
            power: power.clamp(0.05, 1.0),
            fog_start: row.fog_start,
            fog_end: (row.far_clip * FOG_REACH).max(4_000.0),
        })
    }

    /// Unit vector pointing from the world towards the sun, in Bevy axes.
    #[must_use]
    pub fn sun_direction(&self) -> Vec3 {
        let fraction = (self.hour as f32 - 6.0) / 12.0;
        let azimuth = -fraction * std::f32::consts::PI;
        let elevation = (fraction * std::f32::consts::PI).sin().max(0.12) * 75f32.to_radians();
        let horizontal = elevation.cos();
        let game = [
            horizontal * azimuth.cos(),
            horizontal * azimuth.sin(),
            elevation.sin(),
        ];

        to_bevy_position(game)
    }
}

/// Timecyc colours are 0..255 values, authored for an 8-bit framebuffer.
const fn srgb(colour: [f32; 3]) -> Color {
    Color::srgb_u8(
        colour[0].clamp(0.0, 255.0) as u8,
        colour[1].clamp(0.0, 255.0) as u8,
        colour[2].clamp(0.0, 255.0) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sky(hour: usize) -> Sky {
        Sky {
            weather: "SUNNY".to_owned(),
            hour,
            clear: Color::BLACK,
            water: Color::BLACK,
            ambient: Color::BLACK,
            sun: Color::WHITE,
            power: 1.0,
            fog_start: 10.0,
            fog_end: 5_000.0,
        }
    }

    #[test]
    fn puts_the_sun_high_at_noon() {
        let direction = sky(12).sun_direction();

        assert!(direction.y > 0.9, "{direction}");
    }

    #[test]
    fn puts_the_sun_in_the_east_at_sunrise() {
        let direction = sky(6).sun_direction();

        assert!(direction.x > 0.9, "{direction}");
    }

    #[test]
    fn keeps_the_sun_above_the_horizon_at_midnight() {
        assert!(sky(0).sun_direction().y > 0.05);
    }
}
