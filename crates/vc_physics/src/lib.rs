//! Static collision.
//!
//! The world is made of collision *models* placed by instances, so nothing is
//! duplicated: a cell holds instance references, and a query transforms the model's
//! triangles as it tests them. That keeps a whole city's collision in a few
//! megabytes instead of a few hundred.
//!
//! This is the MVP layer: a downward ray for the ground and four horizontal rays for
//! walls. A proper capsule sweep comes when the player can walk; the queries here are
//! the ones a walking player actually needs first.

#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
// Vector maths reads better in its textbook form than in `mul_add` chains, and the
// conventional names for triangle corners and quaternion components are single
// letters. Both are stated once here rather than at every function.
#![expect(
    clippy::suboptimal_flops,
    clippy::many_single_char_names,
    reason = "textbook vector maths: a/b/c corners, x/y/z/w quaternion, plain arithmetic"
)]

use std::collections::HashMap;

use vc_col::{Face, Sphere};

/// Points, in model space, of one collision model.
#[derive(Debug, Clone, Default)]
pub struct Collider {
    /// Vertices.
    pub vertices: Vec<[f32; 3]>,
    /// Faces into those vertices.
    pub faces: Vec<Face>,
    /// Spheres.
    pub spheres: Vec<Sphere>,
    /// Boxes. Vice City keeps a lot of its road and pavement surface in these rather than
    /// in faces, so a query that only walks triangles finds no ground on those streets.
    pub boxes: Vec<vc_col::Box3>,
}

impl Collider {
    /// The model's own bounding box, for placing it in the grid.
    pub fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        let mut min = [f32::INFINITY; 3];
        let mut max = [f32::NEG_INFINITY; 3];

        for vertex in &self.vertices {
            for axis in 0..3 {
                min[axis] = min[axis].min(vertex[axis]);
                max[axis] = max[axis].max(vertex[axis]);
            }
        }

        for sphere in &self.spheres {
            for axis in 0..3 {
                min[axis] = min[axis].min(sphere.centre[axis] - sphere.radius);
                max[axis] = max[axis].max(sphere.centre[axis] + sphere.radius);
            }
        }

        if !min[0].is_finite() {
            return ([0.0; 3], [0.0; 3]);
        }

        (min, max)
    }
}

/// Where a collider sits.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// Translation.
    pub position: [f32; 3],
    /// Orientation as `(x, y, z, w)`.
    pub rotation: [f32; 4],
    /// Scale, applied before rotation.
    pub scale: [f32; 3],
}

impl Placement {
    /// Transforms a model-space point into world space.
    pub fn transform(&self, point: [f32; 3]) -> [f32; 3] {
        let scaled = [
            point[0] * self.scale[0],
            point[1] * self.scale[1],
            point[2] * self.scale[2],
        ];
        let rotated = rotate(scaled, self.rotation);

        [
            rotated[0] + self.position[0],
            rotated[1] + self.position[1],
            rotated[2] + self.position[2],
        ]
    }
}

fn rotate(point: [f32; 3], q: [f32; 4]) -> [f32; 3] {
    let [x, y, z, w] = q;
    // v' = v + 2 * cross(q.xyz, cross(q.xyz, v) + w * v)
    let u = [x, y, z];
    let uv = cross(u, point);
    let uuv = cross(u, uv);
    let uv_w = [uv[0] * w, uv[1] * w, uv[2] * w];

    [
        point[0] + 2.0 * (uuv[0] + uv_w[0]),
        point[1] + 2.0 * (uuv[1] + uv_w[1]),
        point[2] + 2.0 * (uuv[2] + uv_w[2]),
    ]
}

/// Cross product.
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// What one collision model instance references.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Instance {
    collider: usize,
    placement: usize,
}

/// Static collision for a set of placed collision models.
#[derive(Debug, Clone, Default)]
pub struct StaticWorld {
    colliders: Vec<Collider>,
    placements: Vec<Placement>,
    instances: Vec<Instance>,
    grid: HashMap<(i32, i32), Vec<u32>>,
    cell: f32,
}

impl StaticWorld {
    /// Builds a world with the given grid cell size.
    pub fn new(cell: f32) -> Self {
        Self {
            cell,
            ..Self::default()
        }
    }

    /// Registers a collision model and returns its index.
    pub fn add_collider(&mut self, collider: Collider) -> usize {
        self.colliders.push(collider);
        self.colliders.len() - 1
    }

    /// Places a collision model. `bounds` is the model's own bounding box, in model
    /// space; it is transformed by the placement so a rotated or scaled instance is
    /// registered where it actually sits.
    ///
    /// Does nothing when `collider` names no collider or the grid has no cell size,
    /// both of which would otherwise produce a wrong index or an unbounded loop.
    pub fn place(&mut self, collider: usize, placement: Placement, bounds: ([f32; 3], [f32; 3])) {
        if collider >= self.colliders.len() || self.cell <= 0.0 {
            return;
        }

        let placement_index = self.placements.len();
        self.placements.push(placement);

        let instance = self.instances.len() as u32;
        self.instances.push(Instance {
            collider,
            placement: placement_index,
        });

        let (mut min, mut max) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);

        for corner in 0..8 {
            let point = [
                if corner & 1 == 0 {
                    bounds.0[0]
                } else {
                    bounds.1[0]
                },
                if corner & 2 == 0 {
                    bounds.0[1]
                } else {
                    bounds.1[1]
                },
                if corner & 4 == 0 {
                    bounds.0[2]
                } else {
                    bounds.1[2]
                },
            ];
            let world = placement.transform(point);

            for axis in 0..3 {
                min[axis] = min[axis].min(world[axis]);
                max[axis] = max[axis].max(world[axis]);
            }
        }

        // The grid is indexed by ground position, so only x and y matter here.
        for x in cell_range(min[0], max[0], self.cell) {
            for y in cell_range(min[1], max[1], self.cell) {
                self.grid.entry((x, y)).or_default().push(instance);
            }
        }
    }

    /// Highest surface under `point`, searching down `depth` metres.
    ///
    /// The world is Z-up, so the ray falls along `-z` and the returned height is a
    /// `z` value.
    pub fn ground(&self, point: [f32; 3], depth: f32) -> Option<f32> {
        let mut best: Option<f32> = None;

        for instance in self.near(point) {
            let (Some(collider), Some(placement)) = (
                self.colliders.get(instance.collider),
                self.placements.get(instance.placement),
            ) else {
                continue;
            };

            // A downward ray meets the top of a box, which is where the street is on the
            // models built from boxes. Without this the player falls through them.
            for solid in &collider.boxes {
                // All eight corners, not two: a rotated box's opposite corners do not
                // describe its extent, and taking them as one shrinks the surface the
                // player can stand on.
                let (low, high) = box_bounds(placement, solid);
                let top = high[2];
                let floor = low[2];
                let inside = point[0] >= low[0]
                    && point[0] <= high[0]
                    && point[1] >= low[1]
                    && point[1] <= high[1];

                if inside && point[2] >= top && point[2] - top <= depth {
                    best = Some(best.map_or(top, |found| found.max(top)));
                } else if inside && point[2] < top && point[2] >= floor {
                    // Standing inside the solid: the ceiling is the nearest surface up,
                    // which at least stops the fall.
                    best = Some(best.map_or(top, |found| found.max(top)));
                }
            }

            for face in &collider.faces {
                let Some(a) = collider.vertices.get(usize::from(face.indices[0])) else {
                    continue;
                };
                let Some(b) = collider.vertices.get(usize::from(face.indices[1])) else {
                    continue;
                };
                let Some(c) = collider.vertices.get(usize::from(face.indices[2])) else {
                    continue;
                };

                let a = placement.transform(*a);
                let b = placement.transform(*b);
                let c = placement.transform(*c);

                // The cast returns a distance along the ray; the caller wants a height.
                if let Some(distance) = ray_triangle(point, depth, a, b, c) {
                    let height = point[2] - distance;
                    best = Some(best.map_or(height, |found| found.max(height)));
                }
            }

            for sphere in &collider.spheres {
                let centre = placement.transform(sphere.centre);
                let radius = sphere.radius * placement.scale[0].abs();
                let dx = point[0] - centre[0];
                let dy = point[1] - centre[1];

                if dx * dx + dy * dy <= radius * radius {
                    let hit = centre[2] + (radius * radius - dx * dx - dy * dy).sqrt();

                    if hit <= point[2] && point[2] - hit <= depth {
                        best = Some(best.map_or(hit, |found| found.max(hit)));
                    }
                }
            }
        }

        best
    }

    /// Distance from `point` to the nearest surface along `direction`, up to `reach`.
    ///
    /// `direction` is used as given, so pass a unit vector if the result is to be read
    /// as a distance in metres.
    pub fn cast(&self, point: [f32; 3], direction: [f32; 3], reach: f32) -> f32 {
        let mut nearest = reach;

        for instance in self.near(point) {
            let (Some(collider), Some(placement)) = (
                self.colliders.get(instance.collider),
                self.placements.get(instance.placement),
            ) else {
                continue;
            };

            for solid in &collider.boxes {
                let (low, high) = box_bounds(placement, solid);

                if let Some(hit) = ray_box(point, direction, reach, low, high) {
                    nearest = nearest.min(hit);
                }
            }

            for face in &collider.faces {
                let Some(a) = collider.vertices.get(usize::from(face.indices[0])) else {
                    continue;
                };
                let Some(b) = collider.vertices.get(usize::from(face.indices[1])) else {
                    continue;
                };
                let Some(c) = collider.vertices.get(usize::from(face.indices[2])) else {
                    continue;
                };

                let a = placement.transform(*a);
                let b = placement.transform(*b);
                let c = placement.transform(*c);

                if let Some(hit) = ray_triangle_direction(point, direction, reach, a, b, c) {
                    nearest = nearest.min(hit);
                }
            }
        }

        nearest
    }

    fn near(&self, point: [f32; 3]) -> Vec<&Instance> {
        if self.cell <= 0.0 {
            return Vec::new();
        }

        let key = (
            (point[0] / self.cell).floor() as i32,
            (point[1] / self.cell).floor() as i32,
        );

        let mut instances: Vec<u32> = Vec::new();

        for x in -1..=1 {
            for y in -1..=1 {
                if let Some(found) = self.grid.get(&(key.0 + x, key.1 + y)) {
                    instances.extend_from_slice(found);
                }
            }
        }

        instances.sort_unstable();
        instances.dedup();

        instances
            .into_iter()
            .filter_map(|index| self.instances.get(index as usize))
            .collect()
    }
}

/// The world-space axis-aligned bounds of a box, over all eight corners.
fn box_bounds(placement: &Placement, solid: &vc_col::Box3) -> ([f32; 3], [f32; 3]) {
    let mut low = [f32::INFINITY; 3];
    let mut high = [f32::NEG_INFINITY; 3];

    for corner in 0..8u8 {
        let point = [
            if corner & 1 == 0 {
                solid.min[0]
            } else {
                solid.max[0]
            },
            if corner & 2 == 0 {
                solid.min[1]
            } else {
                solid.max[1]
            },
            if corner & 4 == 0 {
                solid.min[2]
            } else {
                solid.max[2]
            },
        ];
        let world = placement.transform(point);

        for axis in 0..3 {
            low[axis] = low[axis].min(world[axis]);
            high[axis] = high[axis].max(world[axis]);
        }
    }

    (low, high)
}

/// Slab test: the distance along `direction` to entering the box, if it is hit at all.
fn ray_box(
    origin: [f32; 3],
    direction: [f32; 3],
    reach: f32,
    low: [f32; 3],
    high: [f32; 3],
) -> Option<f32> {
    let mut enter = 0.0f32;
    let mut exit = reach;

    for axis in 0..3 {
        let start = origin[axis];
        let step = direction[axis];

        if step.abs() < 1e-6 {
            // Parallel to this pair of planes: either always between them or never.
            if start < low[axis] || start > high[axis] {
                return None;
            }

            continue;
        }

        let inverse = step.recip();
        let near = (low[axis] - start) * inverse;
        let far = (high[axis] - start) * inverse;
        let (near, far) = if near <= far {
            (near, far)
        } else {
            (far, near)
        };

        enter = enter.max(near);
        exit = exit.min(far);

        if enter > exit {
            return None;
        }
    }

    Some(enter)
}

fn cell_range(min: f32, max: f32, cell: f32) -> std::ops::RangeInclusive<i32> {
    ((min / cell).floor() as i32)..=((max / cell).floor() as i32)
}

/// Where a downward ray from `origin` hits triangle `abc`, as a distance.
fn ray_triangle(
    origin: [f32; 3],
    depth: f32,
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
) -> Option<f32> {
    ray_triangle_direction(origin, [0.0, 0.0, -1.0], depth, a, b, c)
}

fn ray_triangle_direction(
    origin: [f32; 3],
    direction: [f32; 3],
    reach: f32,
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
) -> Option<f32> {
    let edge1 = sub(b, a);
    let edge2 = sub(c, a);
    let pvec = cross(direction, edge2);
    let det = dot(edge1, pvec);

    if det.abs() < 1e-6 {
        return None;
    }

    let inv_det = 1.0 / det;
    let tvec = sub(origin, a);
    let u = dot(tvec, pvec) * inv_det;

    if !(-1e-4..=1.0 + 1e-4).contains(&u) {
        return None;
    }

    let qvec = cross(tvec, edge1);
    let v = dot(direction, qvec) * inv_det;

    if v < -1e-4 || u + v > 1.0 + 1e-4 {
        return None;
    }

    let t = dot(edge2, qvec) * inv_det;

    (t >= 0.0 && t <= reach).then_some(t)
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// Dot product.
const fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[cfg(test)]
mod tests {
    use super::*;
    use vc_col::Face;

    fn floor() -> Collider {
        Collider {
            vertices: vec![
                [-10.0, -10.0, 0.0],
                [10.0, -10.0, 0.0],
                [10.0, 10.0, 0.0],
                [-10.0, 10.0, 0.0],
            ],
            faces: vec![
                Face {
                    indices: [0, 1, 2],
                    surface: 0,
                },
                Face {
                    indices: [0, 2, 3],
                    surface: 0,
                },
            ],
            spheres: Vec::new(),
            boxes: Vec::new(),
        }
    }

    #[test]
    fn finds_the_floor_under_a_point() {
        let mut world = StaticWorld::new(25.0);
        let collider = world.add_collider(floor());
        world.place(
            collider,
            Placement {
                position: [0.0, 0.0, 0.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0; 3],
            },
            ([-10.0, -10.0, -1.0], [10.0, 10.0, 1.0]),
        );

        assert_eq!(world.ground([0.0, 0.0, 5.0], 10.0), Some(0.0));
        assert_eq!(world.ground([0.0, 20.0, 5.0], 10.0), None);
    }

    #[test]
    fn a_placement_moves_the_floor() {
        let mut world = StaticWorld::new(25.0);
        let collider = world.add_collider(floor());
        world.place(
            collider,
            Placement {
                position: [100.0, 0.0, 12.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0; 3],
            },
            ([-10.0, -10.0, -1.0], [10.0, 10.0, 1.0]),
        );

        assert_eq!(world.ground([100.0, 0.0, 20.0], 20.0), Some(12.0));
    }

    /// A box two units tall, centred on the origin.
    fn block() -> Collider {
        Collider {
            vertices: Vec::new(),
            faces: Vec::new(),
            spheres: Vec::new(),
            boxes: vec![vc_col::Box3 {
                min: [-1.0, -1.0, 0.0],
                max: [1.0, 1.0, 2.0],
                surface: 0,
            }],
        }
    }

    #[test]
    fn a_box_gives_ground_at_its_top() {
        let mut world = StaticWorld::new(25.0);
        let collider = world.add_collider(block());
        world.place(
            collider,
            Placement {
                position: [0.0, 0.0, 0.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0; 3],
            },
            ([-1.0, -1.0, 0.0], [1.0, 1.0, 2.0]),
        );

        assert_eq!(world.ground([0.0, 0.0, 5.0], 10.0), Some(2.0));
    }

    /// Rotated 45 degrees about z, the box's true top is still 2, but its two opposite
    /// corners no longer span its extent, so a two-corner bound gets the width wrong.
    #[test]
    fn a_rotated_box_keeps_its_extent() {
        let mut world = StaticWorld::new(25.0);
        let collider = world.add_collider(block());
        let angle = std::f32::consts::FRAC_PI_4;
        world.place(
            collider,
            Placement {
                position: [0.0, 0.0, 0.0],
                rotation: [0.0, 0.0, (angle * 0.5).sin(), (angle * 0.5).cos()],
                scale: [1.0; 3],
            },
            ([-2.0, -2.0, 0.0], [2.0, 2.0, 2.0]),
        );

        // The corner that reaches furthest along x is now at sqrt(2), so a point at 1.3
        // is over the box and a point at 1.6 is not.
        assert_eq!(world.ground([1.3, 0.0, 5.0], 10.0), Some(2.0));
        assert_eq!(world.ground([1.6, 0.0, 5.0], 10.0), None);
    }

    #[test]
    fn a_cast_is_stopped_by_a_box() {
        let mut world = StaticWorld::new(25.0);
        let collider = world.add_collider(block());
        world.place(
            collider,
            Placement {
                position: [0.0, 0.0, 0.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0; 3],
            },
            ([-1.0, -1.0, 0.0], [1.0, 1.0, 2.0]),
        );

        // Walking east from three units away meets the face at x = -1.
        let hit = world.cast([-3.0, 0.0, 1.0], [1.0, 0.0, 0.0], 5.0);
        assert!((hit - 2.0).abs() < 1e-4, "hit {hit} should be 2");

        // Walking east from above the box passes over it.
        assert_eq!(world.cast([-3.0, 0.0, 9.0], [1.0, 0.0, 0.0], 5.0), 5.0);
    }

    #[test]
    fn a_downward_cast_reports_the_distance_to_the_floor() {
        let mut world = StaticWorld::new(25.0);
        let collider = world.add_collider(floor());
        world.place(
            collider,
            Placement {
                position: [0.0, 0.0, 0.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0; 3],
            },
            ([-10.0, -10.0, -1.0], [10.0, 10.0, 1.0]),
        );

        let hit = world.cast([0.0, 0.0, 4.0], [0.0, 0.0, -1.0], 10.0);

        assert!((hit - 4.0).abs() < 0.001, "expected 4.0, found {hit}");
    }
}
