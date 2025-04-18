use std::collections::HashSet;
use std::time::Duration;

use fmc_client_api as fmc;
use fmc_client_api::{math::BVec3, prelude::*};

// sqrt(2 * gravity * wanted height(1.4)) + some for air resistance
const JUMP_VELOCITY: f32 = 9.0;
const GRAVITY: Vec3 = Vec3::new(0.0, -32.0, 0.0);
// TODO: I think this should be a thing only if you hold space. If you are skilled you can press
// space again as soon as you land if you have released it in the meantime.
// TODO: It feels nice when you jump up a block, but when jumping down it does nothing, feels like
// bouncing. Maybe replace with a jump timer when you land so it's constant? I feel like it would
// be better if you could jump faster when jumping downwards, but not as much as now.
//
// This is needed so that whenever you land early you can't just instantly jump again.
// v_t = v_0 * at => (v_t - v_0) / a = t
const JUMP_TIME: f32 = JUMP_VELOCITY * 1.7 / -GRAVITY.y;

#[derive(Default)]
struct Movement {
    acceleration: Vec3,
    velocity: Vec3,
    is_swimming: bool,
    is_grounded: BVec3,
    is_flying: bool,
    last_spacebar: Duration,
    last_jump: Duration,
    pressed_keys: HashSet<fmc::Key>,
    // Cached delta time
    delta_time: Duration,
}

impl fmc::Plugin for Movement {
    fn update(&mut self) {
        self.delta_time = Duration::from_secs_f32(fmc::delta_time());
        self.last_jump += self.delta_time;
        self.last_spacebar += self.delta_time;

        for key_update in fmc::keyboard_input() {
            if key_update.released {
                if key_update.key == fmc::Key::Space {
                    if self.last_spacebar.as_secs_f32() < 0.25 {
                        self.is_flying = !self.is_flying;
                        self.velocity = Vec3::ZERO;
                    }
                    self.last_spacebar = Duration::default();
                }
                self.pressed_keys.remove(&key_update.key);
            } else {
                self.pressed_keys.insert(key_update.key);
            }
        }

        self.accelerate();
        self.simulate_physics();
    }

    fn set_update_frequency(&mut self) -> Option<f32> {
        Some(1.0 / 60.0)
    }

    fn new() -> Self
    where
        Self: Sized,
    {
        Self::default()
    }
}

fmc::register_plugin!(Movement);

impl Movement {
    fn accelerate(&mut self) {
        let camera_transform = fmc::get_camera_transform();
        let camera_forward = camera_transform.forward();
        let forward = Vec3::new(camera_forward.x, 0., camera_forward.z);
        let sideways = Vec3::new(-camera_forward.z, 0., camera_forward.x);

        if self.is_flying {
            self.velocity.y = 0.0;
        }

        let mut horizontal_acceleration = Vec3::ZERO;
        let mut vertical_acceleration = Vec3::ZERO;
        for key in self.pressed_keys.iter() {
            match *key {
                fmc::Key::KeyW => horizontal_acceleration += forward,
                fmc::Key::KeyS => horizontal_acceleration -= forward,
                fmc::Key::KeyA => horizontal_acceleration -= sideways,
                fmc::Key::KeyD => horizontal_acceleration += sideways,
                fmc::Key::Space => {
                    if self.is_flying {
                        self.velocity.y = JUMP_VELOCITY * 2.0;
                    } else if self.is_swimming {
                        vertical_acceleration.y = 20.0
                    } else if self.is_grounded.y && self.last_jump.as_secs_f32() > JUMP_TIME {
                        self.last_jump = Duration::default();
                        self.velocity.y = JUMP_VELOCITY;
                    }
                }
                fmc::Key::Shift => {
                    if self.is_flying {
                        self.velocity.y = -JUMP_VELOCITY * 2.0;
                    } else if self.is_swimming {
                        vertical_acceleration.y = -30.0
                    }
                }
                _ => (),
            }
        }

        if horizontal_acceleration != Vec3::ZERO {
            horizontal_acceleration = horizontal_acceleration.normalize();
        }

        let mut acceleration = horizontal_acceleration + vertical_acceleration;

        if self.is_flying {
            acceleration *= 140.0;
            if self.pressed_keys.contains(&fmc::Key::Control) {
                acceleration *= 10.0;
            }
        } else if self.is_swimming {
            if acceleration.y == 0.0 {
                acceleration.y = -10.0;
            }
            acceleration.x *= 40.0;
            acceleration.z *= 40.0;
        } else if self.is_grounded.y {
            acceleration *= 100.0;
        } else if self.velocity.x.abs() > 2.0
            || self.velocity.z.abs() > 2.0
            || self.velocity.y < -10.0
        {
            // Move fast in air if you're already in motion
            acceleration *= 50.0;
        } else {
            // Move slow in air in jumping from a standstill
            acceleration *= 20.0;
        }

        if !self.is_flying && !self.is_swimming {
            acceleration += GRAVITY;
        }

        self.acceleration = acceleration;
    }

    // TODO: This tunnels if you move faster than maybe a few blocks a second
    fn simulate_physics(&mut self) {
        let player_transform = fmc::get_player_transform();
        let delta_time = Vec3::splat(self.delta_time.as_secs_f32());

        if self.velocity.x != 0.0 {
            self.is_grounded.x = false;
        }
        if self.velocity.y != 0.0 {
            self.is_grounded.y = false;
        }
        if self.velocity.z != 0.0 {
            self.is_grounded.z = false;
        }

        self.velocity += self.acceleration * delta_time;

        let was_swimming = self.is_swimming;
        self.is_swimming = false;

        let mut new_position = player_transform.translation + self.velocity * delta_time;
        let mut move_back = Vec3::ZERO;
        let mut friction = Vec3::ZERO;
        for velocity in [
            // TODO: If you hit a horizontal ridge at the edge you will pass through because none
            // of these velocity vectors will move the aabb into the block. If a way could be found
            // to not separate the velocity vector into components that would be nice, more
            // efficient too.
            Vec3::new(0.0, self.velocity.y, 0.0),
            Vec3::new(self.velocity.x, 0.0, self.velocity.z),
        ] {
            let pos_after_move = player_transform.translation + velocity * delta_time;

            let player_aabb = Aabb::from_min_max(
                pos_after_move + Vec3::new(-0.3, 0.0, -0.3),
                pos_after_move + Vec3::new(0.3, 1.8, 0.3),
            );

            let start = player_aabb.min().floor().as_ivec3();
            let stop = player_aabb.max().floor().as_ivec3();
            for x in start.x..=stop.x {
                for y in start.y..=stop.y {
                    for z in start.z..=stop.z {
                        let block_pos = IVec3::new(x, y, z);

                        let block_id = match fmc::get_block(block_pos) {
                            Some(id) => id,
                            // Disconnect? Should always have your surroundings loaded.
                            None => return,
                        };

                        let block_friction = fmc::get_block_friction(block_id);

                        friction = friction.max(block_friction.drag);

                        let block_aabb = Aabb {
                            center: block_pos.as_vec3() + 0.5,
                            half_extents: Vec3::new(0.5, 0.5, 0.5),
                        };

                        let Some(overlap) = player_aabb.intersection(&block_aabb) else {
                            continue;
                        };

                        if block_friction.drag.y > 0.4 {
                            self.is_swimming = true;
                        }

                        let backwards_time = overlap / -velocity;
                        let valid_axes = backwards_time.cmplt(delta_time + delta_time / 100.0)
                            & backwards_time.cmpgt(Vec3::splat(0.0));
                        let resolution_axis =
                            Vec3::select(valid_axes, backwards_time, Vec3::NAN).max_element();

                        let Some(surface_friction) = block_friction.surface else {
                            continue;
                        };

                        if resolution_axis == backwards_time.y {
                            move_back.y = overlap.y + overlap.y / 100.0;
                            self.is_grounded.y = true;
                            self.velocity.y = 0.0;

                            if velocity.y.is_sign_positive() {
                                friction = friction.max(Vec3::splat(surface_friction.bottom));
                            } else {
                                friction = friction.max(Vec3::splat(surface_friction.top));
                            }
                        } else if resolution_axis == backwards_time.x {
                            move_back.x = overlap.x + overlap.x / 100.0;
                            self.is_grounded.x = true;
                            self.velocity.x = 0.0;

                            if velocity.x.is_sign_positive() {
                                friction = friction.max(Vec3::splat(surface_friction.left));
                            } else {
                                friction = friction.max(Vec3::splat(surface_friction.right));
                            }
                        } else if resolution_axis == backwards_time.z {
                            move_back.z = overlap.z + overlap.z / 100.0;
                            self.is_grounded.z = true;
                            self.velocity.z = 0.0;

                            if velocity.z.is_sign_positive() {
                                friction = friction.max(Vec3::splat(surface_friction.back));
                            } else {
                                friction = friction.max(Vec3::splat(surface_friction.front));
                            }
                        } else {
                            // When velocity is really small there's numerical precision problems. Since a
                            // resolution is guaranteed. Move it back by whatever the smallest resolution
                            // direction is.
                            let valid_axes = Vec3::select(
                                backwards_time.cmpgt(Vec3::ZERO)
                                    & backwards_time.cmplt(delta_time * 2.0),
                                backwards_time,
                                Vec3::NAN,
                            );
                            if valid_axes.x.is_finite()
                                || valid_axes.y.is_finite()
                                || valid_axes.z.is_finite()
                            {
                                let valid_axes = Vec3::select(
                                    valid_axes.cmpeq(Vec3::splat(valid_axes.min_element())),
                                    valid_axes,
                                    Vec3::ZERO,
                                );
                                move_back += (valid_axes + valid_axes / 100.0) * -velocity;
                            }
                        }
                    }
                }
            }
        }

        new_position += move_back;

        if player_transform.translation != new_position {
            fmc::set_player_transform(Transform {
                translation: new_position,
                rotation: DQuat::IDENTITY,
                scale: Vec3::ONE,
            });
        }

        // XXX: Pow(4) is just to scale it further towards zero when friction is high. The function
        // should be read as 'velocity *= friction^time'
        self.velocity = self.velocity
            * (1.0 - friction)
                .powf(4.0)
                .powf(self.delta_time.as_secs_f32());

        // Give a little boost when exiting water so that the bob stays constant.
        if was_swimming && !self.is_swimming {
            self.velocity.y += 1.5;
        }
    }
}

/// An Axis-Aligned Bounding Box
#[derive(Clone, Debug, Default)]
pub struct Aabb {
    pub center: Vec3,
    pub half_extents: Vec3,
}

impl Aabb {
    #[inline]
    pub fn from_min_max(min: Vec3, max: Vec3) -> Self {
        let min = Vec3::from(min);
        let max = Vec3::from(max);
        let center = 0.5 * (max + min);
        let half_extents = 0.5 * (max - min);
        Self {
            center,
            half_extents,
        }
    }

    #[inline]
    pub fn min(&self) -> Vec3 {
        self.center - self.half_extents
    }

    #[inline]
    pub fn max(&self) -> Vec3 {
        self.center + self.half_extents
    }

    #[inline]
    pub fn intersection(&self, other: &Self) -> Option<Vec3> {
        let distance = self.center - other.center;
        let overlap = self.half_extents + other.half_extents - distance.abs();

        if overlap.cmpgt(Vec3::ZERO).all() {
            // Keep sign to differentiate which side of the block was collided with.
            Some(overlap.copysign(distance))
        } else {
            None
        }
    }
}
