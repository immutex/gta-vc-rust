//! Decides the instance rotation convention from the data itself.
//!
//! Street lights are placed to face the road, and the road graph is independent of
//! the instance transform. So: take each lamp's arm direction in model space, rotate
//! it by the instance quaternion as stored and by its conjugate, and measure which
//! one points at the road. The convention that wins is the one the game used - no
//! eyeballing, no hand-picked locations.
// Diagnostic maths, kept in its textbook form.
#![expect(
    clippy::suboptimal_flops,
    clippy::many_single_char_names,
    reason = "vector maths with conventional single-letter names"
)]

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use vc_paths::Paths;

use vc_transport::GameRoot;
use vc_world::{ObjectDefinition, World};

#[expect(
    clippy::too_many_lines,
    reason = "one linear diagnostic: load, measure, print"
)]
pub(crate) fn run() -> Result<()> {
    let root = GameRoot::from_env()?;
    let (world, world_effects) = load_world(&root.path().join("data/maps"))?;
    let paths = load_paths(&root.path().join("data/maps/paths.ipl"))?;

    // Road *segments*, not nodes: the graph links each point to the next, so the
    // centreline is a polyline and the distance to it is exact. Nearest-node distance
    // is sparse enough to drown the signal.
    let mut segments: Vec<([f32; 3], [f32; 3])> = Vec::new();

    for block in &paths.blocks {
        for (index, point) in block.points.iter().enumerate() {
            if !point.is_in_use() {
                continue;
            }

            if let Some(next) = point.next
                && let Some(target) = block.points.get(usize::from(next))
                && target.is_in_use()
                && usize::from(next) != index
            {
                segments.push((point.position(), target.position()));
            }
        }
    }

    println!("road segments: {}", segments.len());

    let mut definitions: HashMap<u32, ObjectDefinition> =
        HashMap::with_capacity(world.definitions.len());
    for definition in world.definitions {
        let _ = definitions.insert(definition.id, definition);
    }

    // Lamp heads hang over the road, and the 2dfx entry says where the head is in the
    // model's own space. So: place the head under both rotation conventions and see
    // which one lands closer to the road graph. Authored data on both sides.
    let mut effect_directions: HashMap<u32, Vec<[f32; 3]>> = HashMap::new();
    for effect in &world_effects {
        if let Some(direction) = effect.direction
            && direction.iter().any(|value| *value != 0.0)
        {
            effect_directions
                .entry(effect.definition_id)
                .or_default()
                .push(direction);
        }
    }

    let mut effects: HashMap<u32, Vec<[f32; 3]>> = HashMap::new();
    for effect in &world_effects {
        effects
            .entry(effect.definition_id)
            .or_default()
            .push(effect.offset);
    }

    // Shadows are cast in one direction across the whole map. Rotating each model's
    // authored shadow direction into the world must therefore *preserve* that shared
    // direction - and only the correct sense of the instance rotation can. Concentration
    // is the mean resultant length: 1.0 means every shadow agrees.
    let mut shadow_stored = [0.0f64; 3];
    let mut shadow_conjugated = [0.0f64; 3];
    let mut shadow_count = 0u32;

    for instance in &world.instances {
        let Some(offsets) = effects.get(&instance.definition_id) else {
            continue;
        };
        let _ = offsets;
    }

    let mut stored = (0.0f64, 0u32, 0u32);
    let mut conjugated = (0.0f64, 0u32, 0u32);

    for instance in &world.instances {
        let Some(offsets) = effects.get(&instance.definition_id) else {
            continue;
        };
        // Only instances rotated by more than about 16 degrees can distinguish the two
        // readings; for the rest the arms land in the same place either way. And only
        // anchors whose arm is off-axis say anything at all.
        if instance.rotation[3].abs() >= 0.99 {
            continue;
        }

        let post = distance_to_segments(&segments, instance.position);

        if post > 40.0 || post < 0.5 {
            continue;
        }

        for offset in offsets {
            if offset[0].hypot(offset[1]) < 0.5 {
                continue;
            }

            let as_stored = place(*offset, instance.rotation, instance.position);
            let flipped = place(
                *offset,
                [
                    -instance.rotation[0],
                    -instance.rotation[1],
                    -instance.rotation[2],
                    instance.rotation[3],
                ],
                instance.position,
            );

            let to_stored = distance_to_segments(&segments, as_stored);
            let to_flipped = distance_to_segments(&segments, flipped);

            // The sharp test: an arm reaches *over* the road, so the head must end up
            // closer to the centreline than the post it hangs from. That removes the
            // lamp's baseline distance from the comparison entirely.
            stored.0 += f64::from(to_stored);
            stored.1 += 1;
            if to_stored < post {
                stored.2 += 1;
            }

            conjugated.0 += f64::from(to_flipped);
            conjugated.1 += 1;
            if to_flipped < post {
                conjugated.2 += 1;
            }
        }
    }

    for instance in &world.instances {
        let Some(directions) = effect_directions.get(&instance.definition_id) else {
            continue;
        };

        for direction in directions {
            let stored_dir = rotate(*direction, instance.rotation);
            let flipped_dir = rotate(
                *direction,
                [
                    -instance.rotation[0],
                    -instance.rotation[1],
                    -instance.rotation[2],
                    instance.rotation[3],
                ],
            );

            let stored_unit = unit(stored_dir);
            let flipped_unit = unit(flipped_dir);

            for axis in 0..3 {
                shadow_stored[axis] += f64::from(stored_unit[axis]);
                shadow_conjugated[axis] += f64::from(flipped_unit[axis]);
            }
            shadow_count += 1;
        }
    }

    if shadow_count > 0 {
        let stored_len = magnitude(shadow_stored) / f64::from(shadow_count);
        let conjugated_len = magnitude(shadow_conjugated) / f64::from(shadow_count);

        println!(
            "shadow directions: {shadow_count} anchors, concentration stored {stored_len:.3} vs conjugated {conjugated_len:.3}"
        );
        println!("  (1.0 means every shadow points the same way in the world)");
    }

    // Lamps only, and using the game's own arm direction: the 2dfx offset *is* where
    // the head is, so its horizontal part points along the arm. An arm reaches over the
    // kerb, so the arm rotated into the world should point at the nearest road point.
    let mut lamp_stored = (0.0f64, 0u32, 0u32);
    let mut lamp_conjugated = (0.0f64, 0u32, 0u32);

    for instance in &world.instances {
        let Some(definition) = definitions.get(&instance.definition_id) else {
            continue;
        };
        if !definition.model.to_ascii_lowercase().contains("lamp") {
            continue;
        }
        let Some(offsets) = effects.get(&instance.definition_id) else {
            continue;
        };
        let Some((nearest, _)) = nearest_on_segments(&segments, instance.position) else {
            continue;
        };

        let to_road = [
            nearest[0] - instance.position[0],
            nearest[1] - instance.position[1],
        ];

        for offset in offsets {
            let arm = [offset[0], offset[1]];
            let length = arm[0].hypot(arm[1]);

            if length < 0.4 {
                continue;
            }

            for (conjugate, bucket) in [(false, &mut lamp_stored), (true, &mut lamp_conjugated)] {
                let flipped = if conjugate {
                    [
                        -instance.rotation[0],
                        -instance.rotation[1],
                        -instance.rotation[2],
                        instance.rotation[3],
                    ]
                } else {
                    instance.rotation
                };
                let world = rotate([offset[0], offset[1], 0.0], flipped);
                let arm_world = [world[0], world[1]];
                let arm_length = arm_world[0].hypot(arm_world[1]);

                if arm_length < 1e-3 {
                    continue;
                }

                let dot = (arm_world[0] * to_road[0] + arm_world[1] * to_road[1])
                    / (arm_length * to_road[0].hypot(to_road[1]).max(1e-3));

                bucket.0 += f64::from(dot);
                bucket.1 += 1;
                if dot > 0.0 {
                    bucket.2 += 1;
                }
            }
        }
    }

    if lamp_stored.1 > 0 {
        println!(
            "lamps only: {:.1}% of arms point at the road as stored, {:.1}% conjugated",
            100.0 * f64::from(lamp_stored.2) / f64::from(lamp_stored.1),
            100.0 * f64::from(lamp_conjugated.2) / f64::from(lamp_conjugated.1)
        );

        // If the arms were rotated correctly they would all sit near 0 degrees. A peak
        // at 90 means a quarter turn is missing somewhere; a peak at 180 means the
        // model's forward axis is reversed.
        let mut buckets = [0u32; 6];

        for instance in &world.instances {
            let Some(definition) = definitions.get(&instance.definition_id) else {
                continue;
            };
            if !definition.model.to_ascii_lowercase().contains("lamp") {
                continue;
            }
            let Some(offsets) = effects.get(&instance.definition_id) else {
                continue;
            };
            let Some((nearest, _)) = nearest_on_segments(&segments, instance.position) else {
                continue;
            };

            let to_road = [
                nearest[0] - instance.position[0],
                nearest[1] - instance.position[1],
            ];
            let road_length = to_road[0].hypot(to_road[1]);

            if road_length < 1e-3 {
                continue;
            }

            for offset in offsets {
                let world = rotate([offset[0], offset[1], 0.0], instance.rotation);
                let arm_length = world[0].hypot(world[1]);

                if arm_length < 1e-3 {
                    continue;
                }

                let cos = ((world[0] * to_road[0] + world[1] * to_road[1])
                    / (arm_length * road_length))
                    .clamp(-1.0, 1.0);
                let angle = cos.acos().to_degrees();
                let bucket = ((angle / 30.0) as usize).min(5);
                buckets[bucket] += 1;
            }
        }

        println!("arm angle to the road, as stored (every arm counted):");
        for (index, count) in buckets.iter().enumerate() {
            println!("  {:>3}-{:>3} deg: {count}", index * 30, index * 30 + 30);
        }

        // Multi-arm models carry an arm on each side, so the meaningful question is
        // whether *some* arm reaches the road. Best-of-per-instance removes that
        // ambiguity, which is what made the histogram bimodal.
        let mut best_stored = (0u32, 0u32);
        let mut best_conjugated = (0u32, 0u32);

        for instance in &world.instances {
            let Some(definition) = definitions.get(&instance.definition_id) else {
                continue;
            };
            if !definition.model.to_ascii_lowercase().contains("lamp") {
                continue;
            }
            let Some(offsets) = effects.get(&instance.definition_id) else {
                continue;
            };
            let Some((nearest, _)) = nearest_on_segments(&segments, instance.position) else {
                continue;
            };

            let to_road = [
                nearest[0] - instance.position[0],
                nearest[1] - instance.position[1],
            ];
            let road_length = to_road[0].hypot(to_road[1]);

            if road_length < 1e-3 {
                continue;
            }

            let compare = |rotation: [f32; 4]| -> f32 {
                let mut best = f32::NEG_INFINITY;

                for offset in offsets {
                    let world = rotate([offset[0], offset[1], 0.0], rotation);
                    let arm_length = world[0].hypot(world[1]);

                    if arm_length < 1e-3 {
                        continue;
                    }

                    let cos = ((world[0] * to_road[0] + world[1] * to_road[1])
                        / (arm_length * road_length))
                        .clamp(-1.0, 1.0);

                    best = best.max(cos);
                }

                best
            };

            let as_stored = compare(instance.rotation);
            let flipped = compare([
                -instance.rotation[0],
                -instance.rotation[1],
                -instance.rotation[2],
                instance.rotation[3],
            ]);
            let threshold = std::f32::consts::FRAC_1_SQRT_2;

            best_stored.1 += 1;
            best_conjugated.1 += 1;

            if as_stored > threshold {
                best_stored.0 += 1;
            }
            if flipped > threshold {
                best_conjugated.0 += 1;
            }
        }

        if best_stored.1 > 0 {
            println!(
                "best arm within 45 degrees of the road: stored {:.1}%, conjugated {:.1}%",
                100.0 * f32::from(best_stored.0 as u16) / best_stored.1 as f32,
                100.0 * f32::from(best_conjugated.0 as u16) / best_conjugated.1 as f32
            );
        }
    }

    println!("effect anchors measured: {} across the map", stored.1);

    if stored.1 > 0 {
        println!(
            "  stored quaternion: mean {:.2} units from the road",
            stored.0 / f64::from(stored.1)
        );
        println!(
            "  conjugated:        mean {:.2} units from the road",
            conjugated.0 / f64::from(conjugated.1)
        );
        println!(
            "  stored:     {:.1}% of heads land closer to the road than their post",
            100.0 * f64::from(stored.2) / f64::from(stored.1)
        );
        println!(
            "  conjugated: {:.1}% of heads land closer to the road than their post",
            100.0 * f64::from(conjugated.2) / f64::from(conjugated.1)
        );
    }

    Ok(())
}

const fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn unit(v: [f32; 3]) -> [f32; 3] {
    let length = dot(v, v).sqrt();

    if length < 1e-6 {
        return [0.0; 3];
    }

    [v[0] / length, v[1] / length, v[2] / length]
}

fn magnitude(v: [f64; 3]) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

/// Nearest point on the road network, and its distance.
fn nearest_on_segments(
    segments: &[([f32; 3], [f32; 3])],
    point: [f32; 3],
) -> Option<([f32; 3], f32)> {
    let mut best: Option<([f32; 3], f32)> = None;

    for (start, end) in segments {
        let dx = end[0] - start[0];
        let dy = end[1] - start[1];
        let length = dx.mul_add(dx, dy * dy);

        let t = if length < 1e-6 {
            0.0
        } else {
            (((point[0] - start[0]) * dx + (point[1] - start[1]) * dy) / length).clamp(0.0, 1.0)
        };

        let nearest = [dx.mul_add(t, start[0]), dy.mul_add(t, start[1]), start[2]];
        let distance = (point[0] - nearest[0]).hypot(point[1] - nearest[1]);

        if best.is_none_or(|(_, found)| distance < found) {
            best = Some((nearest, distance));
        }
    }

    best
}

/// Horizontal distance from `point` to the nearest road segment.
fn distance_to_segments(segments: &[([f32; 3], [f32; 3])], point: [f32; 3]) -> f32 {
    let mut best = f32::INFINITY;

    for (start, end) in segments {
        let ax = start[0];
        let ay = start[1];
        let bx = end[0];
        let by = end[1];
        let dx = bx - ax;
        let dy = by - ay;
        let length = dx.mul_add(dx, dy * dy);

        let t = if length < 1e-6 {
            0.0
        } else {
            (((point[0] - ax) * dx + (point[1] - ay) * dy) / length).clamp(0.0, 1.0)
        };

        let nearest_x = dx.mul_add(t, ax);
        let nearest_y = dy.mul_add(t, ay);
        let distance = (point[0] - nearest_x).hypot(point[1] - nearest_y);

        if distance < best {
            best = distance;
        }
    }

    best
}

fn rotate(point: [f32; 3], q: [f32; 4]) -> [f32; 3] {
    let [x, y, z, w] = q;
    let u = [x, y, z];
    let uv = cross(u, point);
    let uuv = cross(u, uv);

    [
        point[0] + 2.0 * (uuv[0] + uv[0] * w),
        point[1] + 2.0 * (uuv[1] + uv[1] * w),
        point[2] + 2.0 * (uuv[2] + uv[2] * w),
    ]
}

const fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn load_paths(path: &Path) -> Result<Paths> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let text = String::from_utf8_lossy(&bytes);

    Ok(Paths::parse(&text)?)
}

fn place(offset: [f32; 3], rotation: [f32; 4], position: [f32; 3]) -> [f32; 3] {
    let rotated = rotate(offset, rotation);

    [
        rotated[0] + position[0],
        rotated[1] + position[1],
        rotated[2] + position[2],
    ]
}

fn load_world(directory: &Path) -> Result<(World, Vec<vc_world::Effect>)> {
    let (mut world, effects) = crate::worlds::load_with_effects(directory)?;
    world.instances.retain(|instance| instance.interior <= 0);

    Ok((world, effects))
}
