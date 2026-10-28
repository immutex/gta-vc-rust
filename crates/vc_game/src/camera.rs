//! The free camera, the walking player and the input that drives them.

use bevy::input::mouse::MouseMotion;
use bevy::prelude::*;
use vc_physics::StaticWorld;
use vc_render::to_bevy_position;

/// Gravity in game units per second squared.
pub const GRAVITY: f32 = 10.0;
/// Walking speed in game units per second.
pub const WALK_SPEED: f32 = 6.0;
/// Eye height above the player's feet.
pub const EYE_HEIGHT: f32 = 1.7;
/// Player radius against walls.
pub const PLAYER_RADIUS: f32 = 0.45;
/// Metres per second while flying.
pub const FLY_SPEED: f32 = 60.0;
/// Multiplier while holding shift.
pub const SPRINT: f32 = 4.0;
/// Mouse radians per pixel.
pub const SENSITIVITY: f32 = 0.003;

/// The fly camera's orientation and which mode is active.
#[derive(Resource)]
pub struct FlyCam {
    /// Heading around the world's up axis.
    pub yaw: f32,
    /// Pitch, clamped short of straight up.
    pub pitch: f32,
    /// Whether the player is walking; the camera then follows their eye.
    pub walking: bool,
}

impl FlyCam {
    /// Orientation that looks from `eye` towards `target`.
    #[must_use]
    pub fn looking_at(eye: Vec3, target: Vec3) -> Self {
        let direction = (target - eye).normalize_or_zero();
        let horizontal = direction.x.hypot(direction.z);

        Self {
            yaw: f32::atan2(-direction.x, -direction.z),
            pitch: f32::atan2(direction.y, horizontal),
            walking: false,
        }
    }

    /// The rotation both modes share.
    #[must_use]
    pub fn rotation(&self) -> Quat {
        Quat::from_rotation_y(self.yaw) * Quat::from_rotation_x(self.pitch)
    }
}

/// Marks the camera entity.
#[derive(Component)]
pub struct FlyCamMarker;

/// The player, in game coordinates: Z is up, as the map is authored.
#[derive(Resource)]
pub struct Player {
    /// Feet position.
    pub position: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Whether the feet are on a surface.
    pub on_ground: bool,
}

/// Collision, wrapped so the ECS can hold it.
#[derive(Resource)]
pub struct Physics(pub StaticWorld);

/// Turns the view and switches between walking and flying.
#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters are taken by value by design"
)]
pub fn look(
    keys: Res<'_, ButtonInput<KeyCode>>,
    buttons: Res<'_, ButtonInput<MouseButton>>,
    mut motion: MessageReader<'_, '_, MouseMotion>,
    mut state: ResMut<'_, FlyCam>,
) {
    if keys.just_pressed(KeyCode::KeyF) {
        state.walking = !state.walking;
        println!("mode: {}", if state.walking { "walk" } else { "fly" });
    }

    let looking = buttons.pressed(MouseButton::Left);

    for event in motion.read() {
        if looking {
            state.yaw = f32::mul_add(event.delta.x, -SENSITIVITY, state.yaw);
            state.pitch = f32::mul_add(event.delta.y, -SENSITIVITY, state.pitch).clamp(-1.5, 1.5);
        }
    }
}

/// Moves the camera freely while walking is off.
#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters are taken by value by design"
)]
pub fn fly(
    time: Res<'_, Time>,
    keys: Res<'_, ButtonInput<KeyCode>>,
    state: Res<'_, FlyCam>,
    mut camera: Query<'_, '_, &mut Transform, With<FlyCamMarker>>,
) {
    if state.walking {
        return;
    }

    let Ok(mut transform) = camera.single_mut() else {
        return;
    };

    let rotation = state.rotation();
    transform.rotation = rotation;

    let speed = if keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]) {
        FLY_SPEED * SPRINT
    } else {
        FLY_SPEED
    };
    let forward = rotation * Vec3::NEG_Z;
    let right = rotation * Vec3::X;
    let mut direction = Vec3::ZERO;

    for (key, axis) in [
        (KeyCode::KeyW, forward),
        (KeyCode::KeyS, -forward),
        (KeyCode::KeyD, right),
        (KeyCode::KeyA, -right),
        (KeyCode::Space, Vec3::Y),
        (KeyCode::ControlLeft, Vec3::NEG_Y),
    ] {
        if keys.pressed(key) {
            direction += axis;
        }
    }

    if direction != Vec3::ZERO {
        transform.translation += direction.normalize() * speed * time.delta_secs();
    }
}

/// Moves the player, in game coordinates, and puts the camera at their eye.
#[expect(
    clippy::needless_pass_by_value,
    reason = "Bevy system parameters are taken by value by design"
)]
pub fn walk(
    time: Res<'_, Time>,
    keys: Res<'_, ButtonInput<KeyCode>>,
    state: Res<'_, FlyCam>,
    mut player: ResMut<'_, Player>,
    physics: Res<'_, Physics>,
    mut camera: Query<'_, '_, &mut Transform, With<FlyCamMarker>>,
) {
    if !state.walking {
        return;
    }

    let delta = time.delta_secs().min(0.1);
    let (sin, cos) = state.yaw.sin_cos();
    let forward = Vec3::new(-sin, cos, 0.0);
    let right = Vec3::new(cos, sin, 0.0);
    let mut desired = Vec3::ZERO;

    for (key, axis) in [
        (KeyCode::KeyW, forward),
        (KeyCode::KeyS, -forward),
        (KeyCode::KeyD, right),
        (KeyCode::KeyA, -right),
    ] {
        if keys.pressed(key) {
            desired += axis;
        }
    }

    let speed = if keys.pressed(KeyCode::ShiftLeft) {
        WALK_SPEED * 2.0
    } else {
        WALK_SPEED
    };
    let horizontal = desired.normalize_or_zero() * speed;
    player.velocity.x = horizontal.x;
    player.velocity.y = horizontal.y;

    if player.on_ground && keys.just_pressed(KeyCode::Space) {
        player.velocity.z = 5.5;
        player.on_ground = false;
    }
    player.velocity.z = GRAVITY.mul_add(-delta, player.velocity.z);

    let step = horizontal.length() * delta;
    let mut moved = horizontal;

    if step > 0.0 {
        let origin = player.position + Vec3::Z;
        let reach = step + PLAYER_RADIUS;
        let hit = physics
            .0
            .cast(origin.to_array(), horizontal.normalize().to_array(), reach);

        if hit < reach {
            moved = Vec3::ZERO;
        }
    }

    let fall = player.velocity.z * delta;
    player.position += moved * delta + Vec3::Z * fall;

    let probe = player.position + Vec3::Z * 0.5;
    let ground = physics.0.ground(probe.to_array(), 4.0);
    player.on_ground = false;

    if let Some(height) = ground
        && player.velocity.z <= 0.0
        && player.position.z <= height + 0.4
    {
        player.position.z = height;
        player.velocity.z = 0.0;
        player.on_ground = true;
    }

    // Beaches and props carry no collision in Vice City; float at the water line
    // rather than dropping out of the world.
    if ground.is_none() && player.position.z < 6.0 {
        player.position.z = 6.0;
        player.velocity.z = 0.0;
        player.on_ground = true;
    }

    if let Ok(mut transform) = camera.single_mut() {
        transform.translation =
            to_bevy_position((player.position + Vec3::Z * EYE_HEIGHT).to_array());
        transform.rotation = state.rotation();
    }
}
