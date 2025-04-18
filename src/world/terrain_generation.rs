use fmc::{
    blocks::Blocks,
    noise::{Frequency, Noise},
    // noise::Noise,
    prelude::*,
    world::{
        chunk::{Chunk, ChunkPosition},
        Surface, TerrainGenerator,
    },
};

use rand::SeedableRng;

use super::biomes::Biomes;

pub struct Earth {
    biomes: Biomes,
    continents: Noise,
    terrain_height: Noise,
    terrain_shape: Noise,
    caves: Noise,
    seed: u64,
}

impl TerrainGenerator for Earth {
    fn generate_chunk(&self, chunk_position: ChunkPosition) -> Chunk {
        let mut chunk = Chunk::default();

        let air = Blocks::get().get_id("air");
        const MAX_HEIGHT: i32 = 120;
        if MAX_HEIGHT < chunk_position.y {
            // Don't waste time generating if it is guaranteed to be air.
            chunk.make_uniform(air);
        } else {
            self.generate_terrain(chunk_position, &mut chunk);

            // TODO: Might make sense to test against water too.
            //
            // Test for air chunk uniformity early so we can break and elide the other generation
            // functions. This makes it so all other chunks that are uniform with another type of
            // block get stored as full size chunks. They are assumed to be very rare.
            let mut uniform = true;
            for block in chunk.blocks.iter() {
                if *block != air {
                    uniform = false;
                    break;
                }
            }

            if uniform {
                chunk.make_uniform(air);
                return chunk;
            }

            //self.carve_caves(chunk_position, &mut chunk);
            self.generate_features(chunk_position, &mut chunk);
        }

        return chunk;
    }
}

impl Earth {
    pub fn new(seed: u64, blocks: &Blocks) -> Self {
        let freq = 1.0 / 2f32.powi(9) * 3.0;
        // let freq = 0.00305;
        let continents = Noise::perlin(Frequency {
            x: freq,
            y: 0.0,
            z: freq,
        })
        .seed(seed as u32 + 429340)
        .fbm(4, 0.5, 2.0)
        .abs()
        // This is the "max" height (keep in mind fbm reduces the median amplitude)
        .mul(Noise::constant(120.0))
        // sea
        .add(Noise::constant(-12.0))
        .clamp(-10.0, 10.0);

        //let freq = 1.0 / 2.0f32.powi(5);
        let freq = 0.002189;
        let terrain_height = continents
            .clone()
            .range(
                -2.0,
                2.0,
                Noise::constant(0.0),
                // Noise::constant(1.5),
                Noise::perlin(freq)
                    .seed(seed as u32)
                    .fbm(10, 0.5, 2.0)
                    .mul(Noise::constant(2.0))
                    .add(Noise::constant(1.0)),
            )
            .add(Noise::constant(0.5))
            .clamp(0.5, 1.5);

        let freq = 0.0313;
        let freq = Frequency {
            x: freq,
            y: freq * 1.5,
            z: freq,
        };
        let high = Noise::perlin(freq)
            .seed(seed as u32 + 1239480234)
            .fbm(6, 0.5, 2.0);
        let low = Noise::perlin(freq)
            .seed(seed as u32 + 2239482)
            .fbm(6, 0.5, 2.0);

        // NOTE: Because of interpolation the noise is stretched. 4x horizontally and 8x
        // vertically.
        //
        // High and low are switched between to create sudden changes in terrain elevation.
        //let freq = 0.03379;
        // let freq = 1.0 / 2.0f32.powi(4);
        let terrain_shape = Noise::simplex(Frequency {
            x: freq.x * 1.5,
            y: freq.y * 1.5 * 0.5,
            z: freq.z * 1.5,
        })
        .seed(seed as u32 + 3923480239)
        .fbm(8, 0.5, 2.0)
        .range(0.00, 0.02, low, high);
        // let terrain_shape = high;

        // This is a failed attempt at making snaking tunnels. The idea is to generate 2d noise,
        // abs it, then use the values under some threshold as the direction of the tunnels. To
        // translate it into 3d, a 3d noise is generated through the same procedure, and overlaid
        // on the 2d noise. When you take the absolute value of 3d noise and threshold it, it
        // creates sheets, instead of lines. The overlay between the sheets and the lines of the 2d
        // noise create the tunnels, where the 2d noise effectively constitute the range
        // between the horizontal walls, and the 3d noise the range between the vertical walls.
        //
        // The big problems with this approach is one, no matter which depth you're at, the 2d noise
        // stays the same, and two, the 3d noise creates vertical walls when it changes direction,
        // when the 2d noise is parallel with these walls, it creates really tall narrow
        // unwalkable crevices.
        //
        //let freq = 0.004;
        //let tunnels = Noise::perlin(0.0, seed + 5)
        //    .with_frequency(freq * 2.0, freq * 2.0, freq * 2.0)
        //    .abs()
        //    .max(
        //        Noise::simplex(0.00, seed + 6)
        //            .with_frequency(freq, 0.0, freq)
        //            .abs()
        //    );

        // Visualization: https://www.shadertoy.com/view/stccDB
        // let freq = 0.01;
        // let cave_main = Noise::perlin(fmc_noise::Frequency {
        //     x: freq,
        //     y: freq * 2.0,
        //     z: freq,
        // })
        // .seed(seed as u32 + 5)
        // .fbm(3, 0.5, 2.0)
        // .square();
        // let cave_main_2 = Noise::perlin(fmc_noise::Frequency {
        //     x: freq,
        //     y: freq * 2.0,
        //     z: freq,
        // })
        // .seed(seed as u32 + 6)
        // .fbm(3, 0.5, 2.0)
        // .square();
        // Only generates caves below the continent height so that they're not exposed. I messed up
        // the chunk loading stuff somewhat so when it finds a cave it goes all the way to the
        // bottom of it...
        let caves = continents.clone().range(
            // TODO: These numbers are slightly below the continents max because I implemented
            // range as non-inclusive.
            0.049,
            0.049,
            //cave_main.add(cave_main_2),
            Noise::constant(1.0),
            Noise::constant(1.0),
        );

        Self {
            biomes: Biomes::load(blocks),
            continents,
            terrain_height,
            terrain_shape,
            caves,
            seed,
        }
    }

    fn generate_terrain(&self, chunk_position: ChunkPosition, chunk: &mut Chunk) {
        const WIDTH_FACTOR: usize = 4;
        const HEIGHT_FACTOR: usize = 8;
        const INTERPOLATION_WIDTH: usize = Chunk::SIZE / WIDTH_FACTOR + 1;
        const INTERPOLATION_HEIGHT: usize = Chunk::SIZE / HEIGHT_FACTOR + 2;

        let chunk_x = (chunk_position.x / (WIDTH_FACTOR as i32)) as f32;
        let chunk_y = (chunk_position.y / (HEIGHT_FACTOR as i32)) as f32;
        let chunk_z = (chunk_position.z / (WIDTH_FACTOR as i32)) as f32;
        let (mut terrain, _, _) = self.terrain_shape.generate_3d(
            chunk_x,
            chunk_y,
            chunk_z,
            // More is needed in each direction for interpolation.
            INTERPOLATION_WIDTH,
            INTERPOLATION_HEIGHT,
            INTERPOLATION_WIDTH,
        );

        let (base_height, _, _) =
            self.continents
                .generate_2d(chunk_x, chunk_z, INTERPOLATION_WIDTH, INTERPOLATION_WIDTH);

        let (terrain_height, _, _) = self.terrain_height.generate_2d(
            chunk_x,
            chunk_z,
            INTERPOLATION_WIDTH,
            INTERPOLATION_WIDTH,
        );

        for x in 0..INTERPOLATION_WIDTH {
            for z in 0..INTERPOLATION_WIDTH {
                let index = x * INTERPOLATION_WIDTH + z;
                let base_height = base_height[index];
                let terrain_height = terrain_height[index];
                for y in 0..INTERPOLATION_HEIGHT {
                    // Amount the density should be decreased by per block above the base height.
                    const DECREMENT: f32 = 0.015;
                    let mut compression = ((chunk_position.y + (y * HEIGHT_FACTOR) as i32) as f32
                        - base_height)
                        * DECREMENT
                        / terrain_height;
                    if compression < 0.0 {
                        // Below surface, extra compression
                        compression *= 4.0;
                    }
                    let index = x * (INTERPOLATION_WIDTH * INTERPOLATION_HEIGHT)
                        + z * INTERPOLATION_HEIGHT
                        + y;

                    // Decrease density if above base height, increase if below
                    terrain[index] -= compression;
                }
            }
        }

        let terrain_shape = interpolate(&terrain);

        chunk.blocks = vec![0; Chunk::SIZE.pow(3)];

        let biome = self.biomes.get_biome();

        for x in 0..Chunk::SIZE {
            for z in 0..Chunk::SIZE {
                let mut layer = 0;

                // Find how deep we are from above chunk.
                for y in Chunk::SIZE..CHUNK_HEIGHT {
                    let block_index = x * (Chunk::SIZE * CHUNK_HEIGHT) + z * CHUNK_HEIGHT + y;
                    let density = terrain_shape[block_index];

                    if density <= 0.0 {
                        if chunk_position.y + y as i32 <= 0 {
                            // For water
                            layer = 1;
                        }
                        break;
                    } else {
                        layer += 1;
                    }
                }

                for y in (0..Chunk::SIZE).rev() {
                    let block_height = chunk_position.y + y as i32;

                    let block_index = x * (Chunk::SIZE * CHUNK_HEIGHT) + z * CHUNK_HEIGHT + y;
                    let density = terrain_shape[block_index];

                    let block = if density <= 0.0 {
                        if block_height == 0 {
                            layer = 1;
                            biome.surface_liquid
                        } else if block_height < 0 {
                            layer = 1;
                            biome.sub_surface_liquid
                        } else {
                            layer = 0;
                            biome.air
                        }
                    } else if layer > 3 {
                        layer += 1;
                        biome.bottom_layer_block
                    } else if block_height < 2 {
                        layer += 1;
                        biome.sand
                    } else {
                        let block = if layer < 1 {
                            biome.top_layer_block
                        } else if layer < 3 {
                            biome.mid_layer_block
                        } else {
                            biome.bottom_layer_block
                        };
                        layer += 1;
                        block
                    };

                    chunk[[x, y, z]] = block;
                }
            }
        }
    }

    fn carve_caves(&self, chunk_position: IVec3, chunk: &mut Chunk) {
        let air = Blocks::get().get_id("air");

        let biome = self.biomes.get_biome();
        let (caves, _, _) = self.caves.generate_3d(
            chunk_position.x as f32,
            chunk_position.y as f32,
            chunk_position.z as f32,
            Chunk::SIZE,
            Chunk::SIZE,
            Chunk::SIZE,
        );
        caves
            .into_iter()
            .zip(chunk.blocks.iter_mut())
            .enumerate()
            .for_each(|(i, (mut density, block))| {
                // TODO: Caves and water do not cooperate well. You carve the surface without
                // knowing there's water there and you get reverse moon pools underwater. Instead
                // we just push the caves underground, causing there to be no cave entraces at the
                // surface. There either needs to be a way to exclude caves from being generated
                // beneath water, or some way to intelligently fill carved out space that touches
                // water.
                const DECAY_POINT: i32 = -32;
                let y = chunk_position.y + (i & 0b1111) as i32;
                let density_offset = (y - DECAY_POINT).max(0) as f32 * 1.0 / 64.0;
                density += density_offset;

                if (density / 2.0) < 0.001
                    && *block != biome.surface_liquid
                    && *block != biome.sub_surface_liquid
                {
                    *block = air;
                }
            });
    }

    fn generate_features(&self, chunk_position: ChunkPosition, chunk: &mut Chunk) {
        let blocks = Blocks::get();
        let surface_blocks = [blocks.get_id("grass")];
        let surface = Surface::new(chunk, &surface_blocks, blocks.get_id("air"));

        // x position is left 32 bits and z position the right 32 bits. z must be converted to u32
        // first because it will just fill the left 32 bits with junk. World seed is used to change
        // which chunks are next to each other.
        let seed = ((chunk_position.x as u64) << 32 | chunk_position.z as u32 as u64)
            .overflowing_mul(self.seed)
            .0;
        let mut rng = rand::rngs::StdRng::seed_from_u64(seed);

        let biome = self.biomes.get_biome();

        for blueprint in biome.blueprints.iter() {
            blueprint.construct(chunk_position.into(), chunk, &surface, &mut rng);
        }
    }
}

const CHUNK_HEIGHT: usize = Chunk::SIZE + 8;

// We interpolate from a 4x3x4 to 16x24x16. 24 because we need some of the blocks above the
// chunk to know if we need to place surface blocks. Note how it affects the noise
// frequency. It is effectively 4x(8x vertically) since we sample closer together.
//
// NOTE: This is useful beyond the performance increase.
// 1. 3d noise tends to create small floaters that don't look good.
// 2. Even with complex noise compositions it's very easy to perceive regularity in it.
//    This breaks it up, while providing better continuity.
fn interpolate(noise: &Vec<f32>) -> Vec<f32> {
    const WIDTH: usize = Chunk::SIZE / 4;
    const HEIGHT: usize = CHUNK_HEIGHT / 8;
    const DEPTH: usize = Chunk::SIZE / 4;

    fn index(x: usize, y: usize, z: usize) -> usize {
        return x * (DEPTH + 1) * (HEIGHT + 1) + z * (HEIGHT + 1) + y;
    }

    let mut result = vec![0.0; Chunk::SIZE * CHUNK_HEIGHT * Chunk::SIZE];

    for x_noise in 0..WIDTH {
        for z_noise in 0..DEPTH {
            for y_noise in 0..HEIGHT {
                let mut back_left = noise[index(x_noise + 0, y_noise + 0, z_noise + 0)];
                let mut front_left = noise[index(x_noise + 0, y_noise + 0, z_noise + 1)];
                let mut back_right = noise[index(x_noise + 1, y_noise + 0, z_noise + 0)];
                let mut front_right = noise[index(x_noise + 1, y_noise + 0, z_noise + 1)];
                let back_left_increment =
                    (noise[index(x_noise + 0, y_noise + 1, z_noise + 0)] - back_left) * 0.125;
                let front_left_increment =
                    (noise[index(x_noise + 0, y_noise + 1, z_noise + 1)] - front_left) * 0.125;
                let back_right_increment =
                    (noise[index(x_noise + 1, y_noise + 1, z_noise + 0)] - back_right) * 0.125;
                let front_right_increment =
                    (noise[index(x_noise + 1, y_noise + 1, z_noise + 1)] - front_right) * 0.125;

                for y_index in 0..8 {
                    let y = y_noise * 8 + y_index;

                    let back_increment = (back_right - back_left) * 0.25;
                    let front_increment = (front_right - front_left) * 0.25;

                    let mut back = back_left;
                    let mut front = front_left;

                    for x_index in 0..WIDTH {
                        let x = x_noise * WIDTH + x_index;

                        let bottom_increment = (front - back) * 0.25;
                        let mut density = back;

                        for z_index in 0..DEPTH {
                            let z = z_noise * WIDTH + z_index;
                            result[x * Chunk::SIZE * CHUNK_HEIGHT + z * CHUNK_HEIGHT + y] = density;
                            density += bottom_increment;
                        }

                        back += back_increment;
                        front += front_increment;
                    }

                    back_left += back_left_increment;
                    front_left += front_left_increment;
                    back_right += back_right_increment;
                    front_right += front_right_increment;
                }
            }
        }
    }

    return result;
}
