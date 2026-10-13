package mcgen;

import net.neoforged.bus.api.IEventBus;
import net.neoforged.fml.ModContainer;
import net.neoforged.fml.common.Mod;
import net.neoforged.neoforge.common.NeoForge;

import java.nio.ByteBuffer;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Locale;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;

/**
 * NeoForge entry point for the Rust worldgen engine.
 *
 * <p>At server start this loads the native library, builds a world for a fixed
 * parity seed, runs a self-test and prints the results to the server log. It
 * then registers the {@code /mcgen} command tree, which is how the engine is
 * actually driven — see {@link McGenCommands}.
 *
 * <p>Three design rules worth keeping:
 *
 * <ul>
 *   <li><b>A broken accelerator must not break the server.</b> Every failure path
 *       here logs and returns; nothing throws out of the constructor. A
 *       performance mod that stops a server booting is worse than a slow one.</li>
 *   <li><b>All heavy work is coarse-grained.</b> One call fills many chunks.
 *       There is no per-block callback anywhere in this design.</li>
 *   <li><b>Read-only.</b> Nothing here writes to a world. Chunk generation is
 *       not replaced.</li>
 * </ul>
 *
 * <p>Honest note, repeated in the README: this engine reproduces vanilla's
 * density layer bit for bit but currently runs slower than vanilla's own code.
 * It is shipped as a verifiable, inspectable engine, not as a speedup.
 */
@Mod(McGenMod.MOD_ID)
public class McGenMod {

    public static final String MOD_ID = "mcgen";

    /** Must match {@code MCGEN_ABI_VERSION} in rust/mcgen-ffi/src/lib.rs. */
    public static final int ABI_VERSION = 2;

    static final int OK = 0;
    public static final int BLOCKS_PER_CHUNK = 16 * 16 * 384;
    public static final int HEIGHT = 384;
    public static final int MIN_Y = -64;

    /** The seed the startup self-test uses. Matches the parity fixture's seed. */
    private static final long SELF_TEST_SEED = 1_779_920_288_596_261_407L;

    private static final String LOG = "[mcgen] ";

    /** One native world per seed. Building one parses the density graph, so it is not free. */
    private static final Map<Long, Long> WORLDS = new ConcurrentHashMap<>();

    private static volatile String libraryPath = "(not searched)";
    private static volatile String loadFailure = "not started";

    public McGenMod(IEventBus modEventBus, ModContainer modContainer) {
        try {
            runSelfTest();
        } catch (Throwable t) {
            // Never take the server down because the accelerator misbehaved.
            loadFailure = String.valueOf(t);
            log("self-test failed, continuing without acceleration: " + t);
        }
        try {
            NeoForge.EVENT_BUS.addListener(McGenCommands::register);
            log("commands registered: /mcgen");
        } catch (Throwable t) {
            log("command registration failed (the engine is still usable in the log): " + t);
        }
    }

    // ---- state the commands read ----------------------------------------

    /** True when a native world is loaded and usable. */
    public static boolean isLoaded() {
        return !WORLDS.isEmpty();
    }

    /** Why loading failed, or {@code "ok"} when it did not. */
    public static String loadFailure() {
        return loadFailure;
    }

    /** The path the native library was loaded from. */
    public static String libraryPath() {
        return libraryPath;
    }

    /** A one-line summary for command output. */
    public static String statusSummary() {
        return isLoaded()
                ? "ABI v" + ABI_VERSION + " loaded from " + libraryPath
                : "NOT loaded (" + loadFailure + ")";
    }

    /** The self-test world, which is always built on success. */
    public static long defaultWorld() {
        return WORLDS.getOrDefault(SELF_TEST_SEED, 0L);
    }

    /**
     * Get (or build) an engine for {@code seed}. Returns 0 on failure.
     *
     * <p>Worlds are cached and never freed, on purpose: a server has a small,
     * fixed number of them, and rebuilding one costs a graph parse.
     */
    public static long worldFor(long seed) {
        Long existing = WORLDS.get(seed);
        if (existing != null) {
            return existing;
        }
        synchronized (McGenMod.class) {
            Long again = WORLDS.get(seed);
            if (again != null) {
                return again;
            }
            long h = McGenNative.create(seed);
            if (h != 0L) {
                WORLDS.put(seed, h);
            }
            return h;
        }
    }

    private static void log(String msg) {
        System.out.println(LOG + msg);
    }

    /** Locate the JNI shim: system property, then the server-local folder. */
    private static Path findLibrary() {
        String prop = System.getProperty("mcgen.library");
        if (prop != null && !prop.isBlank()) {
            return Path.of(prop);
        }
        for (String candidate : new String[] {
                "mcgen/libmcgenjni.so",
                "mcgen/libmcgenjni.dylib",
                "mcgen/mcgenjni.dll",
        }) {
            Path p = Path.of(candidate);
            if (Files.exists(p)) {
                return p;
            }
        }
        return null;
    }

    private static long timeNanos(ThrowingRunnable r) throws Exception {
        long t0 = System.nanoTime();
        r.run();
        return System.nanoTime() - t0;
    }

    @FunctionalInterface
    private interface ThrowingRunnable {
        void run() throws Exception;
    }

    private static void runSelfTest() throws Exception {
        Path lib = findLibrary();
        if (lib == null) {
            loadFailure = "native library not found";
            libraryPath = "(not found)";
            log("native library not found (set -Dmcgen.library=<path> or put it in ./mcgen/).");
            log("Skipping acceleration. The /mcgen commands will report this.");
            return;
        }

        libraryPath = lib.toAbsolutePath().toString();
        log("loading " + libraryPath);
        System.load(libraryPath);

        int abi = McGenNative.abiVersion();
        if (abi != ABI_VERSION) {
            loadFailure = "ABI mismatch: library=" + abi + " mod=" + ABI_VERSION;
            log("ABI mismatch: library=" + abi + " mod=" + ABI_VERSION + " — skipping.");
            return;
        }
        log("ABI OK (v" + abi + "), native library live inside NeoForge");

        long world = worldFor(SELF_TEST_SEED);
        if (world == 0L) {
            loadFailure = "world creation failed: " + McGenNative.lastError();
            log("world creation failed: " + McGenNative.lastError());
            return;
        }

        // ---- boundary integrity -------------------------------------
        int n = 4;
        long len = (long) n * n * BLOCKS_PER_CHUNK;
        ByteBuffer buf = ByteBuffer.allocateDirect((int) len);

        long[] hash = new long[1];
        int rc = McGenNative.fingerprint(world, 0, 0, n, hash);
        if (rc != OK) {
            loadFailure = "fingerprint failed rc=" + rc + ": " + McGenNative.lastError();
            log(loadFailure);
            return;
        }
        long rustHash = hash[0];

        rc = McGenNative.fillRegion(world, 0, 0, n, buf);
        if (rc != OK) {
            loadFailure = "fill failed rc=" + rc + ": " + McGenNative.lastError();
            log(loadFailure);
            return;
        }
        long javaHash = fnv1a(buf, len);
        log(String.format(Locale.ROOT,
                "boundary: %,d bytes, rust=0x%016x java=0x%016x %s",
                len, rustHash, javaHash, rustHash == javaHash ? "MATCH" : "MISMATCH"));
        if (rustHash != javaHash) {
            loadFailure = "boundary corrupted data";
            log("boundary corrupted data — refusing to enable acceleration");
            return;
        }

        // ---- parallel speedup ---------------------------------------
        int bn = Integer.getInteger("mcgen.benchChunks", 8);
        long blen = (long) bn * bn * BLOCKS_PER_CHUNK;
        ByteBuffer big = ByteBuffer.allocateDirect((int) blen);

        for (int i = 0; i < 2; i++) {
            McGenNative.fillRegionSerial(world, 0, 0, bn, big);
            McGenNative.fillRegion(world, 0, 0, bn, big);
        }

        long serialBest = Long.MAX_VALUE;
        for (int i = 0; i < 3; i++) {
            serialBest = Math.min(serialBest,
                    timeNanos(() -> McGenNative.fillRegionSerial(world, 0, 0, bn, big)));
        }
        long parallelBest = Long.MAX_VALUE;
        for (int i = 0; i < 3; i++) {
            parallelBest = Math.min(parallelBest,
                    timeNanos(() -> McGenNative.fillRegion(world, 0, 0, bn, big)));
        }

        int chunks = bn * bn;
        double serialCps = chunks / (serialBest / 1e9);
        double parallelCps = chunks / (parallelBest / 1e9);

        // ---- raw noise throughput -----------------------------------
        double[] sum = new double[1];
        long samples = Long.getLong("mcgen.noiseSamples", 10_000_000L);
        McGenNative.noiseSamples(world, 1_000_000L, sum);
        final long sampleCount = samples;
        long noiseNanos = timeNanos(() -> McGenNative.noiseSamples(world, sampleCount, sum));
        double mpps = samples / (noiseNanos / 1e9) / 1e6;

        log(String.format(Locale.ROOT, "fill   : %d chunks  serial %.1f cps | parallel %.1f cps | speedup %.2fx (%d cpus)",
                chunks, serialCps, parallelCps, (double) serialBest / parallelBest,
                Runtime.getRuntime().availableProcessors()));
        log(String.format(Locale.ROOT, "noise  : %,d samples in %.0f ms -> %,.1f M samples/s",
                samples, noiseNanos / 1e6, mpps));

        // One machine-readable line so scripts and CI can assert on it.
        log(String.format(Locale.ROOT,
                "RESULT hash=0x%016x chunks=%d serial_cps=%.1f parallel_cps=%.1f speedup=%.2f noise_mpps=%.1f",
                rustHash, chunks, serialCps, parallelCps,
                (double) serialBest / parallelBest, mpps));

        loadFailure = "ok";
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
