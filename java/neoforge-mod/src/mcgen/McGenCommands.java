package mcgen;

import com.mojang.brigadier.CommandDispatcher;
import com.mojang.brigadier.arguments.IntegerArgumentType;
import com.mojang.brigadier.context.CommandContext;
import net.minecraft.commands.CommandSourceStack;
import net.minecraft.commands.Commands;
import net.minecraft.core.BlockPos;
import net.minecraft.network.chat.Component;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.LevelChunk;
import net.neoforged.neoforge.event.RegisterCommandsEvent;

import java.nio.ByteBuffer;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;

/**
 * The {@code /mcgen} command tree.
 *
 * <p>This is the mod's user interface. Everything it reports comes from the Rust
 * library or from the running server — nothing is hard-coded to make the output
 * look good.
 *
 * <p>The most useful command is {@code /mcgen chunk}. It takes a chunk the
 * server has already loaded, asks the Rust engine to generate that same chunk
 * for the server's own world seed, and compares the two block by block. Run it
 * on a vanilla world and the verdict should match; run it on a world with a
 * worldgen datapack and the mismatches are the datapack's effect, which is
 * exactly what makes it a useful diagnostic rather than a party trick.
 *
 * <p>Two rules, carried over from the mod entry point:
 *
 * <ul>
 *   <li>Nothing here throws out into the server. Every failure is a message.</li>
 *   <li>Nothing here mutates the world. These commands are read-only.</li>
 * </ul>
 */
public final class McGenCommands {

    private McGenCommands() {}

    private static final int CHUNK_SIZE = 16;
    private static final int HEIGHT = McGenMod.HEIGHT;
    private static final int MIN_Y = McGenMod.MIN_Y;
    private static final String PREFIX = "[mcgen] ";

    /** How many mismatching positions to spell out before summarising the rest. */
    private static final int MAX_EXAMPLES = 6;

    public static void register(RegisterCommandsEvent event) {
        CommandDispatcher<CommandSourceStack> d = event.getDispatcher();

        d.register(Commands.literal("mcgen")
                .executes(McGenCommands::help)

                .then(Commands.literal("status")
                        .executes(McGenCommands::status))

                .then(Commands.literal("noises")
                        .executes(McGenCommands::noises))

                .then(Commands.literal("density")
                        .then(Commands.argument("x", IntegerArgumentType.integer())
                                .then(Commands.argument("y", IntegerArgumentType.integer())
                                        .then(Commands.argument("z", IntegerArgumentType.integer())
                                                .executes(McGenCommands::density)))))

                .then(Commands.literal("verify")
                        .requires(s -> s.hasPermission(2))
                        .executes(McGenCommands::verify))

                .then(Commands.literal("bench")
                        .requires(s -> s.hasPermission(2))
                        .executes(c -> bench(c, 4))
                        .then(Commands.argument("chunks", IntegerArgumentType.integer(1, 32))
                                .executes(c -> bench(c, IntegerArgumentType.getInteger(c, "chunks")))))

                .then(Commands.literal("chunk")
                        .requires(s -> s.hasPermission(2))
                        .then(Commands.argument("chunkX", IntegerArgumentType.integer())
                                .then(Commands.argument("chunkZ", IntegerArgumentType.integer())
                                        .executes(McGenCommands::chunk)))));
    }

    // ---- helpers ---------------------------------------------------------

    private static void ok(CommandSourceStack src, String msg) {
        src.sendSuccess(() -> Component.literal(PREFIX + msg), false);
    }

    private static void fail(CommandSourceStack src, String msg) {
        src.sendFailure(Component.literal(PREFIX + msg));
    }

    private static void line(CommandSourceStack src, String msg) {
        ok(src, msg);
    }

    // ---- /mcgen ----------------------------------------------------------

    private static int help(CommandContext<CommandSourceStack> c) {
        CommandSourceStack src = c.getSource();
        line(src, "Rust worldgen engine, " + McGenMod.statusSummary());
        line(src, "/mcgen status            — is the native library live, and for which seed");
        line(src, "/mcgen noises            — list the noise fields the loaded graph references");
        line(src, "/mcgen density <x> <y> <z> — density value at a world position");
        line(src, "/mcgen verify            — re-run the pipeline self-test");
        line(src, "/mcgen bench [chunks]    — chunk-fill throughput, serial and parallel");
        line(src, "/mcgen chunk <cx> <cz>   — compare this engine against a loaded server chunk");
        return 1;
    }

    // ---- /mcgen status ---------------------------------------------------

    private static int status(CommandContext<CommandSourceStack> c) {
        CommandSourceStack src = c.getSource();
        line(src, "library : " + McGenMod.libraryPath());
        line(src, "ABI     : v" + McGenMod.ABI_VERSION + (McGenMod.isLoaded() ? " (loaded)" : " (NOT loaded)"));
        if (!McGenMod.isLoaded()) {
            line(src, "reason  : " + McGenMod.loadFailure());
            return 0;
        }
        long[] seed = new long[1];
        long h = McGenMod.defaultWorld();
        if (McGenNative.seed(h, seed) == McGenMod.OK) {
            line(src, "self-test world seed: " + seed[0]);
        }
        try {
            ServerLevel level = src.getLevel();
            line(src, "this level seed     : " + level.getSeed()
                    + " (a world is built on demand when needed)");
        } catch (Throwable t) {
            line(src, "this level seed     : unavailable (" + t + ")");
        }
        return 1;
    }

    // ---- /mcgen noises ---------------------------------------------------

    private static int noises(CommandContext<CommandSourceStack> c) {
        CommandSourceStack src = c.getSource();
        if (!McGenMod.isLoaded()) {
            fail(src, "native library is not loaded: " + McGenMod.loadFailure());
            return 0;
        }
        long h = McGenMod.defaultWorld();
        int[] count = new int[1];
        if (McGenNative.noiseCount(h, count) != McGenMod.OK) {
            fail(src, "noiseCount failed: " + McGenNative.lastError());
            return 0;
        }
        int n = count[0];
        line(src, n + " noises referenced by the loaded graph:");
        byte[] nameBuf = new byte[256];
        int[] nameLen = new int[1];
        for (int i = 0; i < n; i++) {
            int rc = McGenNative.noiseName(h, i, nameBuf, nameLen);
            if (rc != McGenMod.OK) {
                fail(src, "noiseName(" + i + ") failed: " + McGenNative.lastError());
                return 0;
            }
            line(src, "  [" + i + "] " + new String(nameBuf, 0, nameLen[0], java.nio.charset.StandardCharsets.UTF_8));
        }
        return n;
    }

    // ---- /mcgen density <x> <y> <z> --------------------------------------

    private static int density(CommandContext<CommandSourceStack> c) {
        CommandSourceStack src = c.getSource();
        if (!McGenMod.isLoaded()) {
            fail(src, "native library is not loaded: " + McGenMod.loadFailure());
            return 0;
        }
        int x = IntegerArgumentType.getInteger(c, "x");
        int y = IntegerArgumentType.getInteger(c, "y");
        int z = IntegerArgumentType.getInteger(c, "z");

        long h = McGenMod.defaultWorld();
        double[] out = new double[1];
        if (McGenNative.density(h, x, y, z, out) != McGenMod.OK) {
            fail(src, "density failed: " + McGenNative.lastError());
            return 0;
        }
        double d = out[0];
        line(src, String.format(Locale.ROOT,
                "density at (%d, %d, %d) = %.12f  -> this position is %s",
                x, y, z, d, d > 0.0 ? "solid" : "air"));
        line(src, "note: the unchunked reading. Inside a chunk the `interpolated` markers");
        line(src, "      blend over 4x8x4 cells, so a block near a cell edge can differ.");
        return 1;
    }

    // ---- /mcgen verify ---------------------------------------------------

    /** Re-runs the startup self-test: generate, hash, and check the bytes crossed intact. */
    private static int verify(CommandContext<CommandSourceStack> c) {
        CommandSourceStack src = c.getSource();
        if (!McGenMod.isLoaded()) {
            fail(src, "native library is not loaded: " + McGenMod.loadFailure());
            return 0;
        }
        long h = McGenMod.defaultWorld();
        int n = 2;
        int len = n * n * McGenMod.BLOCKS_PER_CHUNK;
        ByteBuffer buf = ByteBuffer.allocateDirect(len);

        long[] hash = new long[1];
        int rc = McGenNative.fingerprint(h, 0, 0, n, hash);
        if (rc != McGenMod.OK) {
            fail(src, "fingerprint failed: " + McGenNative.lastError());
            return 0;
        }
        rc = McGenNative.fillRegion(h, 0, 0, n, buf);
        if (rc != McGenMod.OK) {
            fail(src, "fill failed: " + McGenNative.lastError());
            return 0;
        }
        long javaHash = fnv1a(buf, len);
        boolean match = javaHash == hash[0];
        line(src, String.format(Locale.ROOT,
                "pipeline: %,d bytes  rust=0x%016x  java=0x%016x  %s",
                (long) len, hash[0], javaHash, match ? "MATCH" : "MISMATCH"));
        if (!match) {
            fail(src, "the bytes did not cross the FFI boundary intact.");
            return 0;
        }
        line(src, "This checks the FFI boundary, not correctness of the terrain. For that,");
        line(src, "run /mcgen chunk on a loaded chunk, which compares against Minecraft itself.");
        return 1;
    }

    // ---- /mcgen bench [chunks] -------------------------------------------

    private static int bench(CommandContext<CommandSourceStack> c, int n) {
        CommandSourceStack src = c.getSource();
        if (!McGenMod.isLoaded()) {
            fail(src, "native library is not loaded: " + McGenMod.loadFailure());
            return 0;
        }
        long h = McGenMod.defaultWorld();
        int len = n * n * McGenMod.BLOCKS_PER_CHUNK;
        int chunks = n * n;

        line(src, "benchmarking " + chunks + " chunks (" + n + "x" + n + ")...");
        try {
            ByteBuffer buf = ByteBuffer.allocateDirect(len);
            // Warm up, so the first timed pass is not paying for page faults.
            McGenNative.fillRegionSerial(h, 0, 0, n, buf);
            McGenNative.fillRegion(h, 0, 0, n, buf);

            long serial = Long.MAX_VALUE;
            for (int i = 0; i < 3; i++) {
                long t0 = System.nanoTime();
                McGenNative.fillRegionSerial(h, 0, 0, n, buf);
                serial = Math.min(serial, System.nanoTime() - t0);
            }
            long parallel = Long.MAX_VALUE;
            for (int i = 0; i < 3; i++) {
                long t0 = System.nanoTime();
                McGenNative.fillRegion(h, 0, 0, n, buf);
                parallel = Math.min(parallel, System.nanoTime() - t0);
            }

            double scps = chunks / (serial / 1e9);
            double pcps = chunks / (parallel / 1e9);
            line(src, String.format(Locale.ROOT,
                    "serial   : %,.1f chunks/s  (%,.1f ms/chunk)", scps, serial / 1e6 / chunks));
            line(src, String.format(Locale.ROOT,
                    "parallel : %,.1f chunks/s  (%,.1f ms/chunk) on %d cpus",
                    pcps, parallel / 1e6 / chunks, Runtime.getRuntime().availableProcessors()));
            line(src, String.format(Locale.ROOT, "speedup  : %.2fx", (double) serial / parallel));
        } catch (OutOfMemoryError e) {
            fail(src, "not enough heap for " + chunks + " chunks (" + (len / (1024 * 1024)) + " MB); try a smaller count.");
            return 0;
        }
        return 1;
    }

    // ---- /mcgen chunk <cx> <cz> ------------------------------------------

    private static int chunk(CommandContext<CommandSourceStack> c) {
        CommandSourceStack src = c.getSource();
        if (!McGenMod.isLoaded()) {
            fail(src, "native library is not loaded: " + McGenMod.loadFailure());
            return 0;
        }
        int cx = IntegerArgumentType.getInteger(c, "chunkX");
        int cz = IntegerArgumentType.getInteger(c, "chunkZ");

        ServerLevel level = src.getLevel();
        long worldSeed = level.getSeed();

        LevelChunk live = level.getChunkSource().getChunkNow(cx, cz);
        if (live == null) {
            fail(src, "chunk " + cx + "," + cz + " is not loaded on this level.");
            line(src, "Move within view distance of it (or forceload it), then run this again.");
            line(src, "This command never generates a chunk itself — only reads what is there.");
            return 0;
        }

        long h = McGenMod.worldFor(worldSeed);
        if (h == 0L) {
            fail(src, "could not build an engine for seed " + worldSeed + ": " + McGenNative.lastError());
            return 0;
        }

        ByteBuffer buf = ByteBuffer.allocateDirect(McGenMod.BLOCKS_PER_CHUNK);
        long t0 = System.nanoTime();
        int rc = McGenNative.fillChunk(h, cx, cz, buf);
        long fillNanos = System.nanoTime() - t0;
        if (rc != McGenMod.OK) {
            fail(src, "fillChunk failed: " + McGenNative.lastError());
            return 0;
        }

        byte[] rust = new byte[McGenMod.BLOCKS_PER_CHUNK];
        buf.get(rust);

        long t1 = System.nanoTime();
        long match = 0;
        long rustSolid = 0;
        long serverSolid = 0;
        List<String> examples = new ArrayList<>();
        Map<String, Integer> serverBlocksAtMismatch = new HashMap<>();
        // Disagreements that are not the density layer's business. Vanilla generates in
        // stages: density decides solid or not, then carvers cut caves, then features plant
        // trees. A tree standing where the density said "air" is not a density error, and
        // saying so is the difference between a score and a diagnosis.
        long mismatchesAfterDensity = 0;

        // Mismatches by depth band. Where they cluster says what is missing: the
        // aquifer stage is not implemented, and aquifers only speak near the
        // water and lava levels, so a bottom-heavy histogram is the signature of
        // that gap rather than of an arithmetic error.
        final int BANDS = 8;
        int bandHeight = (HEIGHT + BANDS - 1) / BANDS;
        long[] bandMismatch = new long[BANDS];
        long[] bandEngineOnly = new long[BANDS];
        long[] bandServerOnly = new long[BANDS];

        for (int y = 0; y < HEIGHT; y++) {
            int wy = MIN_Y + y;
            for (int z = 0; z < CHUNK_SIZE; z++) {
                for (int x = 0; x < CHUNK_SIZE; x++) {
                    int idx = (y * CHUNK_SIZE + z) * CHUNK_SIZE + x;
                    boolean rs = rust[idx] != 0;
                    // The engine answers one question — is the density above zero — so the
                    // thing to compare against is "is this a solid block". Air is not solid,
                    // and neither is water or lava. Treating `!isAir()` as solid counted the
                    // whole ocean as a disagreement; the depth histogram is what made that
                    // visible rather than mysterious.
                    BlockState state = live.getBlockState(new BlockPos(
                            cx * CHUNK_SIZE + x, wy, cz * CHUNK_SIZE + z));
                    boolean ss = !state.isAir() && state.getFluidState().isEmpty();
                    if (rs) {
                        rustSolid++;
                    }
                    if (ss) {
                        serverSolid++;
                    }
                    if (rs == ss) {
                        match++;
                    } else {
                        int band = y / bandHeight;
                        if (band >= BANDS) {
                            band = BANDS - 1;
                        }
                        bandMismatch[band]++;
                        if (rs) {
                            bandEngineOnly[band]++;
                        } else {
                            bandServerOnly[band]++;
                        }
                    }
                    if (rs != ss) {
                        String serverBlock = blockName(state);
                        serverBlocksAtMismatch.merge(serverBlock, 1, Integer::sum);
                        if (isLaterStageBlock(serverBlock)) {
                            mismatchesAfterDensity++;
                        }
                        if (examples.size() < MAX_EXAMPLES) {
                            examples.add(String.format(Locale.ROOT,
                                    "  (%d, %d, %d): engine=%s server=%s",
                                    cx * CHUNK_SIZE + x, wy, cz * CHUNK_SIZE + z,
                                    rs ? "solid" : "air", serverBlock));
                        }
                    }
                }
            }
        }
        long scanNanos = System.nanoTime() - t1;

        long total = (long) McGenMod.BLOCKS_PER_CHUNK;
        long mismatches = total - match;
        double pct = 100.0 * match / total;

        line(src, String.format(Locale.ROOT,
                "chunk %d,%d  seed %d  —  %,d of %,d positions agree (%.3f%%)",
                cx, cz, worldSeed, match, total, pct));
        line(src, String.format(Locale.ROOT,
                "  solid (non-air, non-fluid): engine %,d  server %,d   |  fill %,.1f ms, scan %,.1f ms",
                rustSolid, serverSolid, fillNanos / 1e6, scanNanos / 1e6));

        if (mismatches == 0) {
            line(src, "The engine's solid/air verdict matches this chunk exactly.");
        } else {
            line(src, "mismatches by depth (engine-only = engine says solid, server air):");
            for (int b = 0; b < BANDS; b++) {
                if (bandMismatch[b] == 0) {
                    continue;
                }
                int lo = MIN_Y + b * bandHeight;
                int hi = Math.min(MIN_Y + (b + 1) * bandHeight, MIN_Y + HEIGHT) - 1;
                line(src, String.format(Locale.ROOT,
                        "  y %5d..%5d : %,7d  (engine-only %,d / server-only %,d)",
                        lo, hi, bandMismatch[b], bandEngineOnly[b], bandServerOnly[b]));
            }

            for (String e : examples) {
                line(src, e);
            }
            if (mismatches > examples.size()) {
                line(src, "  ... and " + (mismatches - examples.size()) + " more");
            }
            // Naming the blocks on the server side turns a bare count into a diagnosis.
            List<Map.Entry<String, Integer>> top = new ArrayList<>(serverBlocksAtMismatch.entrySet());
            top.sort((a, b) -> Integer.compare(b.getValue(), a.getValue()));
            StringBuilder sb = new StringBuilder("server block at those positions: ");
            for (int i = 0; i < Math.min(4, top.size()); i++) {
                if (i > 0) {
                    sb.append(", ");
                }
                sb.append(top.get(i).getKey()).append(" x").append(top.get(i).getValue());
            }
            line(src, sb.toString());

            long densityMismatch = mismatches - mismatchesAfterDensity;
            line(src, String.format(Locale.ROOT,
                    "of those, %,d are blocks a later stage placed (trees, carved cave air) and",
                    mismatchesAfterDensity));
            line(src, String.format(Locale.ROOT,
                    "%,d are the density layer's own: %.3f%% agreement there.",
                    densityMismatch, 100.0 * (total - densityMismatch) / total));
            line(src, "This engine implements the density layer only. It does not carve caves, place");
            line(src, "ores, run aquifers or apply surface rules, so disagreements of the first kind");
            line(src, "are expected. Disagreements of the second kind would be a bug — report those.");
        }
        return mismatches == 0 ? 1 : 0;
    }

    /**
     * True for blocks produced by a generation stage that runs after density.
     *
     * <p>Two families, deliberately kept crude: vegetation (features plant it) and
     * air/void air (carvers and aquifers remove material). Anything else -- stone,
     * dirt, water, lava -- is something the density layer itself is responsible for,
     * so it counts against us. Erring toward "this is our fault" is the useful
     * direction for a diagnostic to err in.
     */
    private static boolean isLaterStageBlock(String name) {
        if (name.equals("minecraft:air") || name.equals("minecraft:cave_air")
                || name.equals("minecraft:void_air")) {
            return true;
        }
        for (String suffix : new String[] {
                "_leaves", "_log", "_wood", "_sapling", "_planks",
        }) {
            if (name.endsWith(suffix)) {
                return true;
            }
        }
        return false;
    }

    /** The registry name of a block, e.g. {@code minecraft:water}. */
    private static String blockName(BlockState state) {
        try {
            return net.minecraft.core.registries.BuiltInRegistries.BLOCK
                    .getKey(state.getBlock()).toString();
        } catch (Throwable t) {
            return state.getBlock().toString();
        }
    }

    /** FNV-1a 64, matching {@code mcgen_core::fnv1a_bytes}. */
    private static long fnv1a(ByteBuffer buf, long len) {
        long hash = 0xcbf29ce484222325L;
        for (long i = 0; i < len; i++) {
            hash ^= (buf.get((int) i) & 0xFFL);
            hash *= 0x100000001b3L;
        }
        return hash;
    }
}
