# Licence notice

**SPDX-License-Identifier: GPL-3.0-or-later**

This project is licensed under the GNU General Public License v3.0 or later.

The full licence text should be added as `LICENSE` before publishing. It is
available at <https://www.gnu.org/licenses/gpl-3.0.txt> or from the FSF.

## Why GPL-3.0

The worldgen noise implementation was written by studying
[Pumpkin](https://github.com/Pumpkin-MC/Pumpkin), specifically
`crates/pumpkin-util/src/noise/perlin.rs`, which is a Rust port of Minecraft's
`ImprovedNoise`, `PerlinNoise` and `NormalNoise` classes. Pumpkin is licensed
GPL-3.0, and this project reproduces its algorithmic structure — octave layout,
normalization constants, gradient table, and the "legacy fBm" construction.

## Deliberate divergence from Pumpkin

Our implementation keeps everything in `f64` to match vanilla's `double` math.
Pumpkin converts to `f32` early (its gradient table is `[[f32; 3]; 16]`). This is
a correctness choice for a vanilla-parity goal — and it means our output will
*not* match Pumpkin's parity fingerprints, so Pumpkin cannot be used as a
cross-check oracle.

## Other prior art worth crediting

Not copied, but valuable references for the same problem:

- [tritium / libworldgen](https://github.com/Bumseltag/tritium) — Rust, GPU-accelerated
  worldgen, with a datapack loader and a standalone worldgen previewer.
- [worldgen-rs](https://github.com/garfxld/worldgen-rs) — faithful Rust port of vanilla
  worldgen, currently reaching `ChunkStatus::Surface`.
- [tungsten-mc-compile](https://docs.rs/tungsten-mc-compile/latest/tungsten_mc_compile/) —
  compiles Minecraft datapack density functions to optimized Rust/CUDA.
- [Oxidizium](https://modrinth.com/mod/oxidizium) — Fabric mod replacing vanilla methods
  with Rust natives, demonstrating the in-JVM approach.
- [C2ME](https://github.com/RelativityMC/C2ME-fabric) — the mature Java implementation of
  parallel chunk generation. The best available reference for *what* to accelerate.

Minecraft is a trademark of Mojang Studios. This project is not affiliated with
or endorsed by Mojang or NeoForged.
