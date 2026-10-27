//! Reader for `data/timecyc.dat`.
//!
//! The file is the master table behind Vice City's weather and lighting: seven weather types, each
//! with one row per hour of the day, and every row carrying 52 values that describe ambient light,
//! the directional sun, the sky gradient, fog, clouds and water.
//!
//! The column order below is the order the file's own header comment names, and it accounts for
//! exactly the 52 values each row holds.
//!
//! # Examples
//!
//! ```
//! use vc_timecyc::Timecycle;
//!
//! let row = "10 10 10  20 20 20  10 10 10  10 10 10  255 255 255  1 1 1  2 2 2  3 3 3  4 4 4 \
//!            1.0 1.0 1.0 200 100 0 2000.0 100.0 1.0  30 20 0  23 32 33  3 3 3  5 3 0  85 85 65 192";
//! let text = format!("//// WEATHER\n{}\n", vec![row; 24].join("\n"));
//!
//! let table = Timecycle::parse(&text)?;
//!
//! assert_eq!(table.weathers[0].name, "WEATHER");
//! assert_eq!(table.weathers[0].hours.len(), 24);
//! # Ok::<(), vc_timecyc::TimecycError>(())
//! ```

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

use thiserror::Error;

/// Values in one row of the table.
pub const VALUES_PER_ROW: usize = 52;

/// Rows per weather type.
pub const HOURS_PER_WEATHER: usize = 24;

/// Slashes that introduce a weather section, as opposed to a `//` comment.
pub const SECTION_MARKER_SLASHES: usize = 4;

/// One hour of one weather type, in the order the file stores it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hour {
    /// Ambient light colour.
    pub ambient: [f32; 3],
    /// Ambient light applied to objects.
    pub ambient_object: [f32; 3],
    /// Ambient light applied to buildings.
    pub ambient_buildings: [f32; 3],
    /// Ambient light applied to objects on buildings.
    pub ambient_object_buildings: [f32; 3],
    /// Directional light colour.
    pub directional: [f32; 3],
    /// Top of the sky gradient.
    pub sky_top: [f32; 3],
    /// Bottom of the sky gradient.
    pub sky_bottom: [f32; 3],
    /// Sun core colour.
    pub sun_core: [f32; 3],
    /// Sun corona colour.
    pub sun_corona: [f32; 3],
    /// Sun sprite size.
    pub sun_size: f32,
    /// Corona sprite size.
    pub sprite_size: f32,
    /// Corona sprite brightness.
    pub sprite_brightness: f32,
    /// Shadow intensity.
    pub shadow: f32,
    /// Intensity of shadows cast by light.
    pub light_shadow: f32,
    /// Intensity of shadows cast by poles.
    pub pole_shadow: f32,
    /// Far clip plane in world units.
    pub far_clip: f32,
    /// Fog start distance.
    pub fog_start: f32,
    /// Whether light pools on the ground.
    pub light_on_ground: f32,
    /// Low cloud colour.
    pub low_clouds: [f32; 3],
    /// Top cloud colour.
    pub top_clouds: [f32; 3],
    /// Bottom cloud colour.
    pub bottom_clouds: [f32; 3],
    /// Blur colour applied to the frame.
    pub blur: [f32; 3],
    /// Water colour and alpha.
    pub water: [f32; 4],
}

/// One weather type: a name and 24 hours.
#[derive(Debug, Clone, PartialEq)]
pub struct Weather {
    /// Weather name from the section header, for example `SUNNY`.
    pub name: String,
    /// One entry per hour, starting at midnight.
    pub hours: Vec<Hour>,
}

/// The whole table.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Timecycle {
    /// Weather types in file order.
    pub weathers: Vec<Weather>,
}

impl Timecycle {
    /// Parses the table.
    ///
    /// # Errors
    /// Returns [`TimecycError::FieldCount`] when a row does not hold
    /// [`VALUES_PER_ROW`] values, and [`TimecycError::HourCount`] when a weather
    /// type does not hold [`HOURS_PER_WEATHER`] rows.
    pub fn parse(text: &str) -> Result<Self, TimecycError> {
        let mut table = Self::default();
        let mut current: Option<Weather> = None;

        for line in text.lines() {
            let trimmed = line.trim();

            if trimmed.is_empty() {
                continue;
            }

            if trimmed.starts_with("//") {
                // Every section repeats the column names as a `//` comment, so
                // only a longer slash run introduces a weather type.
                let slashes = trimmed.chars().take_while(|c| *c == '/').count();

                if slashes >= SECTION_MARKER_SLASHES {
                    let name = trimmed.trim_start_matches('/').trim();

                    table.close(&mut current, table.weathers.len())?;
                    current = Some(Weather {
                        name: name.to_owned(),
                        hours: Vec::with_capacity(HOURS_PER_WEATHER),
                    });
                }

                continue;
            }

            let Some(weather) = current.as_mut() else {
                continue;
            };

            weather.hours.push(parse_hour(
                trimmed,
                table.weathers.len(),
                weather.hours.len(),
            )?);
        }

        table.close(&mut current, table.weathers.len())?;

        Ok(table)
    }

    fn close(&mut self, current: &mut Option<Weather>, index: usize) -> Result<(), TimecycError> {
        let Some(weather) = current.take() else {
            return Ok(());
        };

        if weather.hours.len() != HOURS_PER_WEATHER {
            return Err(TimecycError::HourCount {
                weather: index,
                name: weather.name,
                found: weather.hours.len(),
            });
        }

        self.weathers.push(weather);

        Ok(())
    }

    /// Every hour across every weather type.
    pub fn hours(&self) -> impl Iterator<Item = &Hour> {
        self.weathers
            .iter()
            .flat_map(|weather| weather.hours.iter())
    }

    /// Narrowest far clip plane in the table, in world units.
    pub fn far_clip_range(&self) -> Option<(f32, f32)> {
        let mut range: Option<(f32, f32)> = None;

        for hour in self.hours() {
            range = Some(match range {
                Some((min, max)) => (min.min(hour.far_clip), max.max(hour.far_clip)),
                None => (hour.far_clip, hour.far_clip),
            });
        }

        range
    }
}

fn parse_hour(line: &str, weather: usize, hour: usize) -> Result<Hour, TimecycError> {
    let values: Vec<f32> = line
        .split_whitespace()
        .filter_map(|field| field.parse::<f32>().ok())
        .collect();

    if values.len() != VALUES_PER_ROW {
        return Err(TimecycError::FieldCount {
            weather,
            hour,
            found: values.len(),
        });
    }

    let mut reader = Reader {
        values: &values,
        cursor: 0,
    };

    let hour_values = Hour {
        ambient: reader.take3(),
        ambient_object: reader.take3(),
        ambient_buildings: reader.take3(),
        ambient_object_buildings: reader.take3(),
        directional: reader.take3(),
        sky_top: reader.take3(),
        sky_bottom: reader.take3(),
        sun_core: reader.take3(),
        sun_corona: reader.take3(),
        sun_size: reader.take1(),
        sprite_size: reader.take1(),
        sprite_brightness: reader.take1(),
        shadow: reader.take1(),
        light_shadow: reader.take1(),
        pole_shadow: reader.take1(),
        far_clip: reader.take1(),
        fog_start: reader.take1(),
        light_on_ground: reader.take1(),
        low_clouds: reader.take3(),
        top_clouds: reader.take3(),
        bottom_clouds: reader.take3(),
        blur: reader.take3(),
        water: reader.take4(),
    };

    Ok(hour_values)
}

/// Walks a row's values in the order the file stores them.
struct Reader<'a> {
    values: &'a [f32],
    cursor: usize,
}

impl Reader<'_> {
    fn take1(&mut self) -> f32 {
        let value = self.values[self.cursor];
        self.cursor += 1;

        value
    }

    fn take3(&mut self) -> [f32; 3] {
        [self.take1(), self.take1(), self.take1()]
    }

    fn take4(&mut self) -> [f32; 4] {
        [self.take1(), self.take1(), self.take1(), self.take1()]
    }
}

/// Failures while reading the table.
#[derive(Debug, Error, PartialEq)]
pub enum TimecycError {
    /// A row does not carry the expected number of values.
    #[error("weather {weather} hour {hour} has {found} values, expected {VALUES_PER_ROW}")]
    FieldCount {
        /// Weather index.
        weather: usize,
        /// Hour index.
        hour: usize,
        /// Values found.
        found: usize,
    },
    /// A weather type does not carry 24 rows.
    #[error("weather {weather} (`{name}`) has {found} rows, expected {HOURS_PER_WEATHER}")]
    HourCount {
        /// Weather index.
        weather: usize,
        /// Weather name.
        name: String,
        /// Rows found.
        found: usize,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> &'static str {
        "10 11 12  20 21 22  30 31 32  40 41 42  255 255 255  50 51 52  60 61 62  70 71 72 \
         80 81 82  1.5 2.5 3.5  200 100 7  2000.0 100.0 1.0  90 91 92  23 32 33  3 3 3 \
         5 3 0  85 85 65 192"
    }

    fn weather(name: &str, rows: usize) -> String {
        let mut text = format!("//// {name}\n");

        for _ in 0..rows {
            text.push_str(row());
            text.push('\n');
        }

        text
    }

    #[test]
    fn reads_one_weather_with_24_rows() {
        let table = Timecycle::parse(&weather("SUNNY", 24)).unwrap();

        assert_eq!(table.weathers.len(), 1);
        assert_eq!(table.weathers[0].name, "SUNNY");
        assert_eq!(table.weathers[0].hours.len(), HOURS_PER_WEATHER);
    }

    #[test]
    fn reads_the_columns_in_the_order_the_file_declares() {
        let table = Timecycle::parse(&weather("SUNNY", 24)).unwrap();
        let hour = table.weathers[0].hours[0];

        assert_eq!(hour.ambient, [10.0, 11.0, 12.0]);
        assert_eq!(hour.directional, [255.0, 255.0, 255.0]);
        assert_eq!(hour.sky_top, [50.0, 51.0, 52.0]);
        assert_eq!(hour.sun_size, 1.5);
        assert_eq!(hour.far_clip, 2000.0);
        assert_eq!(hour.fog_start, 100.0);
        assert_eq!(hour.water, [85.0, 85.0, 65.0, 192.0]);
    }

    #[test]
    fn reads_several_weather_types() {
        let text = format!("{}{}", weather("SUNNY", 24), weather("RAINY", 24));

        let table = Timecycle::parse(&text).unwrap();

        assert_eq!(table.weathers.len(), 2);
        assert_eq!(table.weathers[1].name, "RAINY");
    }

    #[test]
    fn reports_the_far_clip_range() {
        let table = Timecycle::parse(&weather("SUNNY", 24)).unwrap();

        assert_eq!(table.far_clip_range(), Some((2000.0, 2000.0)));
    }

    #[test]
    fn rejects_a_row_with_too_few_values() {
        let text = "//// SUNNY\n10 11 12\n";

        let result = Timecycle::parse(text);

        assert!(matches!(
            result,
            Err(TimecycError::FieldCount { found: 3, .. })
        ));
    }

    #[test]
    fn rejects_a_weather_with_the_wrong_number_of_rows() {
        let result = Timecycle::parse(&weather("SUNNY", 5));

        assert!(matches!(
            result,
            Err(TimecycError::HourCount { found: 5, .. })
        ));
    }

    #[test]
    fn ignores_the_column_comment_that_every_section_repeats() {
        let text = format!("//// SUNNY\n// Amb Obj Dir\n{}", vec![row(); 24].join("\n"));

        let table = Timecycle::parse(&text).unwrap();

        assert_eq!(table.weathers.len(), 1);
    }

    #[test]
    fn ignores_lines_before_the_first_header() {
        let text = format!("some preamble\n{}", weather("SUNNY", 24));

        let table = Timecycle::parse(&text).unwrap();

        assert_eq!(table.weathers.len(), 1);
    }
}
