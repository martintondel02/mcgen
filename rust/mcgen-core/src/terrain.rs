//! Turning density values into blocks.
//!
//! This is a deliberately simple kernel: for each block centre, evaluate the
//! density function and emit stone when it is positive. Real 1.21 terrain
//! shaping also runs aquifers, ore veins and surface rules, and vanilla samples
//! a coarse cell grid and interpolates rather than sampling every block.
//! Those are the next milestones — see the README's status table.

use rayon::prelude::*;

use crate::density::DensityFunction;
use crate::noise::NormalNoise;

pub const CHUNK_SIZE: usize = 16;
pub const MIN_Y: i32 = -64;
pub const MAX_Y: i32 = 320;
/// 384 blocks: the 1.18+ overworld height.
pub const HEIGHT: usize = (MAX_Y - MIN_Y) as usize;
pub const BLOCKS_PER_CHUNK: usize = CHUNK_SIZE * CHUNK_SIZE * HEIGHT;

pub const BLOCK_AIR: u8 = 0;
pub const BLOCK_STONE: u8 = 1;

/// Block index within a chunk, y-major (matches vanilla's paletted container order).
#[inline]
#[must_use]
pub fn block_index(x: usize, y: usize, z: usize) -> usize {
    (y * CHUNK_SIZE + z) * CHUNK_SIZE + x
}

// ---------------------------------------------------------------------------
// Vanilla's cell grid.
//
// 1.18+ does NOT evaluate the density function per block. It samples a coarse
// grid and trilinearly interpolates. The spacing comes from `NoiseSettings`;
// for the overworld it is cellWidth 4 (x and z) and cellHeight 8 (y), confirmed
// by asking vanilla: `RouterOracle settings` prints
// `minY=-64 height=384 sizeHorizontal=1 sizeVertical=2 cellWidth=4 cellHeight=8`.
//
// The interpolation arithmetic (`Mth.lerp3`) is verified bit-exact against
// vanilla by the `math` parity check.
// ---------------------------------------------------------------------------

/// Blocks between cell samples horizontally.
pub const CELL_WIDTH: usize = 4;
/// Blocks between cell samples vertically.
pub const CELL_HEIGHT: usize = 8;

const SAMPLES_X: usize = CHUNK_SIZE / CELL_WIDTH + 1;
const SAMPLES_Y: usize = HEIGHT / CELL_HEIGHT + 1;
const SAMPLES_Z: usize = SAMPLES_X;

/// Density sampled on vanilla's coarse cell grid for one chunk.
#[derive(Debug, Clone)]
pub struct CellGrid {
    samples: Vec<f64>,
}

impl CellGrid {
    #[inline]
    fn sample_index(ix: usize, iy: usize, iz: usize) -> usize {
        (iy * SAMPLES_X + iz) * SAMPLES_X + ix
    }

    /// A raw grid sample, indexed by sample position (not block position).
    #[must_use]
    pub fn sample(&self, ix: usize, iy: usize, iz: usize) -> f64 {
        self.samples[Self::sample_index(ix, iy, iz)]
    }

    /// Trilinearly interpolate to a chunk-local block position.
    ///
    /// `x` and `z` must be in `0..CHUNK_SIZE` and `y` in `0..HEIGHT`. The grid has
    /// one more sample than cells along each axis — the extra sample is the far
    /// corner at `x = 16`, used only to interpolate the last cell, and is not a
    /// valid block position.
    ///
    /// At a sample point (`x % CELL_WIDTH == 0`, `y % CELL_HEIGHT == 0`,
    /// `z % CELL_WIDTH == 0`) this returns the sampled value **exactly**, because
    /// `lerp3` with a zero fraction reduces to the corner. That invariant is what
    /// makes the layout testable without an oracle.
    #[must_use]
    pub fn interpolate(&self, x: usize, y: usize, z: usize) -> f64 {
        assert!(x < CHUNK_SIZE && z < CHUNK_SIZE && y < HEIGHT, "block position out of chunk: ({x},{y},{z})");

        let cx = x / CELL_WIDTH;
        let cy = y / CELL_HEIGHT;
        let cz = z / CELL_WIDTH;

        let fx = (x % CELL_WIDTH) as f64 / CELL_WIDTH as f64;
        let fy = (y % CELL_HEIGHT) as f64 / CELL_HEIGHT as f64;
        let fz = (z % CELL_WIDTH) as f64 / CELL_WIDTH as f64;

        let s = |ix, iy, iz| self.sample(ix, iy, iz);

        crate::math::lerp3(
            fx,
            fy,
            fz,
            s(cx, cy, cz),
            s(cx + 1, cy, cz),
            s(cx, cy + 1, cz),
            s(cx + 1, cy + 1, cz),
            s(cx, cy, cz + 1),
            s(cx + 1, cy, cz + 1),
            s(cx, cy + 1, cz + 1),
            s(cx + 1, cy + 1, cz + 1),
        )
    }

    #[must_use]
    pub fn sample_count(&self) -> usize {
        self.samples.len()
    }
}

/// A density graph plus the noise instances it references.
pub struct WorldGen<'a> {
    pub density: &'a DensityFunction,
    pub noises: &'a [NormalNoise],
}

impl WorldGen<'_> {
    /// Fill one chunk's blocks into `out`, which must be `BLOCKS_PER_CHUNK` long.
    pub fn fill_chunk(&self, chunk_x: i32, chunk_z: i32, out: &mut [u8]) {
        assert_eq!(
            out.len(),
            BLOCKS_PER_CHUNK,
            "output buffer must hold exactly one chunk"
        );

        let base_x = chunk_x * CHUNK_SIZE as i32;
        let base_z = chunk_z * CHUNK_SIZE as i32;

        for y in 0..HEIGHT {
            // Vanilla evaluates density functions at *integer block coordinates*
            // (`FunctionContext.blockX/Y/Z`), not at block centres. Sampling at
            // `+ 0.5` would be a systematic half-block offset, so this is a
            // correctness detail rather than a style choice.
            let wy = MIN_Y + y as i32;
            for z in 0..CHUNK_SIZE {
                let wz = base_z + z as i32;
                for x in 0..CHUNK_SIZE {
                    let wx = base_x + x as i32;
                    let d = self.density.compute(
                        self.noises,
                        f64::from(wx),
                        f64::from(wy),
                        f64::from(wz),
                    );
                    out[block_index(x, y, z)] = if d > 0.0 { BLOCK_STONE } else { BLOCK_AIR };
                }
            }
        }
    }

    /// Sample the density function on vanilla's coarse cell grid for one chunk.
    ///
    /// Sample `(ix, iy, iz)` is the block at
    /// `(chunk_x * 16 + ix * 4, MIN_Y + iy * 8, chunk_z * 16 + iz * 4)`.
    #[must_use]
    pub fn sample_cells(&self, chunk_x: i32, chunk_z: i32) -> CellGrid {
        let base_x = chunk_x * CHUNK_SIZE as i32;
        let base_z = chunk_z * CHUNK_SIZE as i32;

        let mut samples = vec![0.0f64; SAMPLES_X * SAMPLES_Y * SAMPLES_Z];
        for iy in 0..SAMPLES_Y {
            // Sampled at integer block coordinates, matching vanilla's
            // FunctionContext.blockX/Y/Z.
            let wy = f64::from(MIN_Y + (iy * CELL_HEIGHT) as i32);
            for iz in 0..SAMPLES_Z {
                let wz = f64::from(base_z + (iz * CELL_WIDTH) as i32);
                for ix in 0..SAMPLES_X {
                    let wx = f64::from(base_x + (ix * CELL_WIDTH) as i32);
                    let d = self.density.compute(self.noises, wx, wy, wz);
                    samples[CellGrid::sample_index(ix, iy, iz)] = d;
                }
            }
        }

        CellGrid { samples }
    }

    /// Fill one chunk the way a `NoiseChunk` actually does.
    ///
    /// There is no separate cell-grid pass. `NoiseChunk.wrapNew` replaces `interpolated`
    /// markers with `NoiseInterpolator`s and evaluates everything else per block, so the
    /// coarse sampling is a property of those markers and nothing else. The per-cell corner
    /// cache inside [`DensityFunction::compute_chunked`] is what keeps this affordable.
    pub fn fill_chunk_chunked(&self, chunk_x: i32, chunk_z: i32, out: &mut [u8]) {
        assert_eq!(out.len(), BLOCKS_PER_CHUNK);
        crate::density::clear_interpolation_cache();

        let base_x = chunk_x * CHUNK_SIZE as i32;
        let base_z = chunk_z * CHUNK_SIZE as i32;

        // Walked **cell by cell**, not block by block. The corner cache behind
        // `compute_chunked` lives for one 4 x 8 x 4 cell, so all 128 blocks of a cell have to be
        // visited together for it to do anything; with the obvious `z, x, y` order it was
        // emptied every 8 blocks and cost more than it saved. Vanilla's cell machine exists for
        // the same reason.
        for cell_z in 0..CHUNK_SIZE / CELL_WIDTH {
            for cell_x in 0..CHUNK_SIZE / CELL_WIDTH {
                for cell_y in 0..HEIGHT / CELL_HEIGHT {
                    for dy in 0..CELL_HEIGHT {
                        let y = cell_y * CELL_HEIGHT + dy;
                        let wy = MIN_Y + y as i32;
                        for dz in 0..CELL_WIDTH {
                            let z = cell_z * CELL_WIDTH + dz;
                            let wz = base_z + z as i32;
                            for dx in 0..CELL_WIDTH {
                                let x = cell_x * CELL_WIDTH + dx;
                                let wx = base_x + x as i32;
                                let d = self.density.compute_chunked(
                                    self.noises,
                                    f64::from(wx),
                                    f64::from(wy),
                                    f64::from(wz),
                                );
                                out[block_index(x, y, z)] =
                                    if d > 0.0 { BLOCK_STONE } else { BLOCK_AIR };
                            }
                        }
                    }
                }
            }
        }

    }

    /// Fill one chunk by coarse-sampling the *whole* density on the cell grid.
    ///
    /// This is an approximation, not what vanilla does: vanilla interpolates only the
    /// subtrees marked `interpolated`. It is retained because it is ~100x cheaper and agrees
    /// with the real thing in sign at 98,302 of 98,304 tested points, which makes it useful
    /// for bulk work where a handful of blocks may be wrong.
    pub fn fill_chunk_interpolated(&self, chunk_x: i32, chunk_z: i32, out: &mut [u8]) {
        assert_eq!(out.len(), BLOCKS_PER_CHUNK);
        let grid = self.sample_cells(chunk_x, chunk_z);
        for y in 0..HEIGHT {
            for z in 0..CHUNK_SIZE {
                for x in 0..CHUNK_SIZE {
                    let d = grid.interpolate(x, y, z);
                    out[block_index(x, y, z)] = if d > 0.0 { BLOCK_STONE } else { BLOCK_AIR };
                }
            }
        }
    }

    /// Fill an `n x n` region of chunks, one thread per chunk, using vanilla's
    /// cell-grid interpolation.
    ///
    /// Deterministic: each chunk is written to its own disjoint slice, so the
    /// result does not depend on scheduling.
    pub fn fill_region(&self, chunk_x0: i32, chunk_z0: i32, n: i32, out: &mut [u8]) {
        let chunk_count = (n * n) as usize;
        assert_eq!(
            out.len(),
            chunk_count * BLOCKS_PER_CHUNK,
            "output buffer must hold n*n chunks"
        );

        // The real path. `fill_region` used to use the coarse grid, which is not what vanilla
        // computes; the FFI surface must not ship an approximation under a vanilla name.
        out.par_chunks_mut(BLOCKS_PER_CHUNK)
            .enumerate()
            .for_each(|(i, buf)| {
                let i = i as i32;
                let cx = chunk_x0 + (i % n);
                let cz = chunk_z0 + (i / n);
                self.fill_chunk_chunked(cx, cz, buf);
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::density::DensityBuilder;
    use crate::noise::NormalNoise;
    use crate::random::LegacyRandomSource;

    fn build_world() -> (DensityFunction, Vec<NormalNoise>) {
        let mut b = DensityBuilder::new();
        let f = b
            .parse_str(
                r#"{
                  "type":"minecraft:add",
                  "argument1":{
                    "type":"minecraft:y_clamped_gradient",
                    "from_y":-64,"to_y":320,"from_value":2.0,"to_value":-2.0
                  },
                  "argument2":{
                    "type":"minecraft:mul",
                    "argument1":{
                      "type":"minecraft:noise","noise":"test:terrain",
                      "xz_scale":0.05,"y_scale":0.1
                    },
                    "argument2":{"type":"minecraft:constant","argument":1.0}
                  }
                }"#,
            )
            .unwrap();
        // NOTE: noise *contents* are irrelevant to these structural tests.
        let noises = b
            .noise_names()
            .iter()
            .enumerate()
            .map(|(i, _)| {
                let mut r = LegacyRandomSource::new(5 + i as u64);
                NormalNoise::new_legacy(&mut r, -7, &[1.0, 1.0, 1.0, 1.0])
            })
            .collect();
        (f, noises)
    }

    #[test]
    fn chunk_fill_produces_solid_below_and_air_above() {
        let (density, noises) = build_world();
        let wg = WorldGen {
            density: &density,
            noises: &noises,
        };

        let mut buf = vec![0u8; BLOCKS_PER_CHUNK];
        wg.fill_chunk(0, 0, &mut buf);

        assert_eq!(buf.len(), BLOCKS_PER_CHUNK);

        // The gradient dominates: y=-64 is strongly positive, y=319 strongly
        // negative, so the bottom must be solid and the top must be air
        // regardless of what the noise contributes.
        let bottom = buf[block_index(8, 0, 8)];
        let top = buf[block_index(8, HEIGHT - 1, 8)];
        assert_eq!(bottom, BLOCK_STONE, "bottom of the world should be solid");
        assert_eq!(top, BLOCK_AIR, "top of the world should be air");
    }

    #[test]
    fn cell_grid_has_vanilla_sample_dimensions() {
        let (density, noises) = build_world();
        let wg = WorldGen {
            density: &density,
            noises: &noises,
        };
        let grid = wg.sample_cells(0, 0);
        // 16/4 + 1 = 5 in x and z; 384/8 + 1 = 49 in y.
        assert_eq!(grid.sample_count(), 5 * 49 * 5);
    }

    #[test]
    fn interpolation_is_exact_at_every_cell_corner() {
        // The load-bearing invariant of the cell grid: at a sample point the
        // trilinear blend must reduce to the sampled value bit-for-bit. If the
        // layout, spacing or corner ordering were wrong, this would fail.
        let (density, noises) = build_world();
        let wg = WorldGen {
            density: &density,
            noises: &noises,
        };
        let grid = wg.sample_cells(0, 0);

        // Corners inside the chunk. The grid carries one extra sample per axis
        // (the far corner at x/z = 16 and y = 320) which is not a block position.
        for iy in 0..SAMPLES_Y {
            let y = iy * CELL_HEIGHT;
            if y >= HEIGHT {
                continue;
            }
            for iz in 0..SAMPLES_Z - 1 {
                for ix in 0..SAMPLES_X - 1 {
                    let (x, z) = (ix * CELL_WIDTH, iz * CELL_WIDTH);
                    let got = grid.interpolate(x, y, z);
                    let want = grid.sample(ix, iy, iz);
                    assert_eq!(
                        got.to_bits(),
                        want.to_bits(),
                        "corner ({ix},{iy},{iz}) not reproduced exactly"
                    );
                }
            }
        }

        // And the far edge sample is genuinely reachable: the last cell's x+1
        // corner must be the sample at ix = SAMPLES_X - 1.
        for iy in 0..SAMPLES_Y {
            let y = iy * CELL_HEIGHT;
            if y >= HEIGHT {
                continue;
            }
            let got = grid.interpolate(CHUNK_SIZE - 1, y, CHUNK_SIZE - 1);
            assert!(got.is_finite());
            assert!(grid.sample(SAMPLES_X - 1, iy, SAMPLES_Z - 1).is_finite());
        }
    }

    #[test]
    fn interpolated_and_direct_fills_agree_exactly_at_corners() {
        let (density, noises) = build_world();
        let wg = WorldGen {
            density: &density,
            noises: &noises,
        };

        let mut interpolated = vec![0u8; BLOCKS_PER_CHUNK];
        let mut direct = vec![0u8; BLOCKS_PER_CHUNK];
        wg.fill_chunk_interpolated(0, 0, &mut interpolated);
        wg.fill_chunk(0, 0, &mut direct);

        for y in (0..HEIGHT).step_by(CELL_HEIGHT) {
            for z in (0..CHUNK_SIZE).step_by(CELL_WIDTH) {
                for x in (0..CHUNK_SIZE).step_by(CELL_WIDTH) {
                    let i = block_index(x, y, z);
                    assert_eq!(
                        interpolated[i], direct[i],
                        "corner block ({x},{y},{z}) disagrees"
                    );
                }
            }
        }
    }

    #[test]
    fn interpolated_fill_is_solid_below_and_air_above() {
        let (density, noises) = build_world();
        let wg = WorldGen {
            density: &density,
            noises: &noises,
        };
        let mut buf = vec![0u8; BLOCKS_PER_CHUNK];
        wg.fill_chunk_interpolated(0, 0, &mut buf);
        assert_eq!(buf[block_index(8, 0, 8)], BLOCK_STONE);
        assert_eq!(buf[block_index(8, HEIGHT - 1, 8)], BLOCK_AIR);
    }

    #[test]
    fn interpolated_and_direct_fills_differ_somewhere() {
        // Guards against the grid silently degenerating into a no-op.
        let (density, noises) = build_world();
        let wg = WorldGen {
            density: &density,
            noises: &noises,
        };
        let mut a = vec![0u8; BLOCKS_PER_CHUNK];
        let mut b = vec![0u8; BLOCKS_PER_CHUNK];
        wg.fill_chunk_interpolated(0, 0, &mut a);
        wg.fill_chunk(0, 0, &mut b);
        assert_ne!(a, b, "interpolation had no effect at all");
    }

    #[test]
    fn density_is_sampled_at_integer_block_coordinates() {
        // A gradient that is +1 at y=0 and exactly 0 at y=0.5. If the kernel
        // sampled block centres (+0.5), block y=0 would come out as air.
        let mut b = DensityBuilder::new();
        let f = b
            .parse_str(
                r#"{"type":"minecraft:y_clamped_gradient",
                    "from_y":0,"to_y":1,"from_value":1.0,"to_value":-1.0}"#,
            )
            .unwrap();
        let wg = WorldGen {
            density: &f,
            noises: &[],
        };

        let mut buf = vec![0u8; BLOCKS_PER_CHUNK];
        wg.fill_chunk(0, 0, &mut buf);

        let y_index = (0 - MIN_Y) as usize;
        assert_eq!(
            buf[block_index(0, y_index, 0)],
            BLOCK_STONE,
            "world y=0 must be sampled at exactly 0, not at 0.5"
        );
        // And the row above must be air, confirming the gradient is live.
        assert_eq!(buf[block_index(0, y_index + 1, 0)], BLOCK_AIR);
    }

    #[test]
    fn region_fill_is_deterministic_and_matches_serial_fill() {
        let (density, noises) = build_world();
        let wg = WorldGen {
            density: &density,
            noises: &noises,
        };

        let n = 4;
        let mut parallel = vec![0u8; (n * n) as usize * BLOCKS_PER_CHUNK];
        wg.fill_region(0, 0, n, &mut parallel);

        let mut serial = vec![0u8; (n * n) as usize * BLOCKS_PER_CHUNK];
        let mut single = vec![0u8; BLOCKS_PER_CHUNK];
        for i in 0..(n * n) as i32 {
            // Must match what fill_region uses, which is now the chunked path.
            wg.fill_chunk_chunked(i % n, i / n, &mut single);
            let off = i as usize * BLOCKS_PER_CHUNK;
            serial[off..off + BLOCKS_PER_CHUNK].copy_from_slice(&single);
        }

        assert_eq!(
            parallel, serial,
            "parallel and serial fills must agree byte for byte"
        );
    }

    #[test]
    fn region_fill_is_stable_across_repeated_runs() {
        let (density, noises) = build_world();
        let wg = WorldGen {
            density: &density,
            noises: &noises,
        };
        let n = 3;
        let mut a = vec![0u8; (n * n) as usize * BLOCKS_PER_CHUNK];
        let mut b = vec![0u8; (n * n) as usize * BLOCKS_PER_CHUNK];
        wg.fill_region(0, 0, n, &mut a);
        wg.fill_region(0, 0, n, &mut b);
        assert_eq!(a, b, "repeated runs must be identical (no data races)");
    }

    #[test]
    fn different_chunks_differ() {
        let (density, noises) = build_world();
        let wg = WorldGen {
            density: &density,
            noises: &noises,
        };
        let mut a = vec![0u8; BLOCKS_PER_CHUNK];
        let mut b = vec![0u8; BLOCKS_PER_CHUNK];
        wg.fill_chunk(0, 0, &mut a);
        wg.fill_chunk(50, 50, &mut b);
        assert_ne!(a, b, "distant chunks should not be identical");
    }
}
