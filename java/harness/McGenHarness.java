import java.lang.foreign.Arena;
import java.lang.foreign.FunctionDescriptor;
import java.lang.foreign.Linker;
import java.lang.foreign.MemorySegment;
import java.lang.foreign.SymbolLookup;
import java.lang.foreign.ValueLayout;
import java.lang.invoke.MethodHandle;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Locale;

/**
 * End-to-end harness for the Rust worldgen accelerator.
 *
 * <p>This is M0/M1 of the plan: it proves three things that the whole design
 * depends on, and nothing else.
 *
 * <ol>
 *   <li><b>The boundary works.</b> A real Rust {@code cdylib} loads into a JVM
 *       through {@code java.lang.foreign} and its functions are callable.</li>
 *   <li><b>Data crosses faithfully.</b> Java hashes the off-heap buffer Rust
 *       filled and compares it to the hash Rust computed internally. If the
 *       marshalling were wrong, these would disagree.</li>
 *   <li><b>It is actually fast.</b> Parallel vs serial fill of the same region,
 *       and raw noise throughput, both measured.</li>
 * </ol>
 *
 * <p>What it deliberately does <i>not</i> prove: that the generated terrain
 * matches vanilla. That needs the Xoroshiro splitter path and real
 * noise_settings data. See the README.
 *
 * <p>Java 21's FFM API is a preview feature, so this must be compiled and run
 * with {@code --enable-preview}, plus {@code --enable-native-access} for the
 * restricted operations.
 */
public final class McGenHarness {

    // ---- ABI contract, mirrored from rust/mcgen-ffi/src/lib.rs -------------
    static final int ABI_VERSION = 1;
    static final int OK = 0;
    static final int ERR_NULL = 1;
    static final int ERR_LEN = 2;
    static final int ERR_ARG = 3;
    static final int ERR_JSON = 4;
    static final int ERR_PANIC = -1;

    static final int BLOCKS_PER_CHUNK = 16 * 16 * 384;

    // ---- native bindings ---------------------------------------------------
    final MethodHandle abiVersion;
    final MethodHandle lastError;
    final MethodHandle worldCreate;
    final MethodHandle worldDestroy;
    final MethodHandle fillRegion;
    final MethodHandle fillRegionSerial;
    final MethodHandle fingerprint;
    final MethodHandle noiseSamples;

    McGenHarness(Path libPath) {
        Linker linker = Linker.nativeLinker();
        SymbolLookup lib = SymbolLookup.libraryLookup(libPath, Arena.global());

        abiVersion = down(linker, lib, "mcgen_abi_version",
                FunctionDescriptor.of(ValueLayout.JAVA_INT));
        lastError = down(linker, lib, "mcgen_last_error",
                FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.JAVA_LONG));
        worldCreate = down(linker, lib, "mcgen_world_create",
                FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.JAVA_LONG, ValueLayout.ADDRESS));
        worldDestroy = down(linker, lib, "mcgen_world_destroy",
                FunctionDescriptor.ofVoid(ValueLayout.ADDRESS));
        fillRegion = down(linker, lib, "mcgen_world_fill_region",
                FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS,
                        ValueLayout.JAVA_INT, ValueLayout.JAVA_INT, ValueLayout.JAVA_INT,
                        ValueLayout.ADDRESS, ValueLayout.JAVA_LONG));
        fillRegionSerial = down(linker, lib, "mcgen_world_fill_region_serial",
                FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS,
                        ValueLayout.JAVA_INT, ValueLayout.JAVA_INT, ValueLayout.JAVA_INT,
                        ValueLayout.ADDRESS, ValueLayout.JAVA_LONG));
        fingerprint = down(linker, lib, "mcgen_world_fingerprint",
                FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS,
                        ValueLayout.JAVA_INT, ValueLayout.JAVA_INT, ValueLayout.JAVA_INT,
                        ValueLayout.ADDRESS));
        noiseSamples = down(linker, lib, "mcgen_world_noise_samples",
                FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS,
                        ValueLayout.JAVA_LONG, ValueLayout.ADDRESS));
    }

    private static MethodHandle down(Linker linker, SymbolLookup lib, String name,
                                      FunctionDescriptor desc) {
        return linker.downcallHandle(
                lib.find(name).orElseThrow(() -> new IllegalStateException(
                        "native symbol not found: " + name)),
                desc);
    }

    // ---- helpers -----------------------------------------------------------
    static final class CheckFailure extends RuntimeException {
        CheckFailure(String msg) { super(msg); }
    }

    static void check(boolean condition, String message) {
        if (!condition) throw new CheckFailure(message);
    }

    String errorMessage(Arena arena) {
        try {
            MemorySegment buf = arena.allocate(512);
            int n = (int) lastError.invokeExact(buf, 512L);
            // Java 21 name; this became MemorySegment.getString in Java 22.
            return n > 0 ? buf.getUtf8String(0) : "(no message)";
        } catch (Throwable t) {
            return "(failed to read error: " + t + ")";
        }
    }

    /** FNV-1a 64 over a segment, matching {@code mcgen_core::fnv1a_bytes}. */
    static long fnv1a(MemorySegment seg, long len) {
        long hash = 0xcbf29ce484222325L;
        for (long i = 0; i < len; i++) {
            hash ^= (seg.get(ValueLayout.JAVA_BYTE, i) & 0xFFL);
            hash *= 0x100000001b3L;
        }
        return hash;
    }

    static String fmt(long nanos) {
        return String.format(Locale.ROOT, "%,.2f ms", nanos / 1e6);
    }

    // ---- main --------------------------------------------------------------
    public static void main(String[] args) throws Throwable {
        Path libPath = Path.of(args.length > 0 ? args[0]
                : "rust/target/release/libmcgen.so");
        check(Files.exists(libPath), "native library not found at " + libPath.toAbsolutePath());

        System.out.println("=== mcgen harness ===");
        System.out.println("library : " + libPath.toAbsolutePath());
        System.out.println("java    : " + System.getProperty("java.version"));
        System.out.println("cpus    : " + Runtime.getRuntime().availableProcessors());
        System.out.println();

        McGenHarness h = new McGenHarness(libPath);

        try (Arena arena = Arena.ofConfined()) {
            // --- 1. ABI handshake ------------------------------------------
            int abi = (int) h.abiVersion.invokeExact();
            System.out.println("[1] ABI");
            System.out.println("    native ABI version : " + abi);
            check(abi == ABI_VERSION,
                    "ABI mismatch: library says " + abi + ", harness expects " + ABI_VERSION);
            System.out.println("    PASS: ABI matches");

            // --- 2. create a world ----------------------------------------
            long seed = 1_779_920_288_596_261_407L;
            MemorySegment handleSlot = arena.allocate(ValueLayout.ADDRESS);
            int rc = (int) h.worldCreate.invokeExact(seed, handleSlot);
            check(rc == OK, "mcgen_world_create failed rc=" + rc + ": " + h.errorMessage(arena));
            MemorySegment world = handleSlot.get(ValueLayout.ADDRESS, 0);
            check(!world.equals(MemorySegment.NULL), "world handle was NULL");
            System.out.println();
            System.out.println("[2] world created for seed " + seed);

            // --- 3. boundary integrity ------------------------------------
            // Rust writes terrain into a Java-allocated off-heap buffer and
            // reports its own hash. Java re-hashes the bytes it can see.
            System.out.println();
            System.out.println("[3] boundary integrity");
            int n = 4;
            long len = (long) n * n * BLOCKS_PER_CHUNK;
            MemorySegment buf = arena.allocate(len);

            MemorySegment hashSlot = arena.allocate(ValueLayout.JAVA_LONG);
            rc = (int) h.fingerprint.invokeExact(world, 0, 0, n, hashSlot);
            check(rc == OK, "fingerprint failed rc=" + rc + ": " + h.errorMessage(arena));
            long rustHash = hashSlot.get(ValueLayout.JAVA_LONG, 0);

            rc = (int) h.fillRegion.invokeExact(world, 0, 0, n, buf, len);
            check(rc == OK, "fill_region failed rc=" + rc + ": " + h.errorMessage(arena));

            long javaHash = fnv1a(buf, len);
            System.out.printf("    region        : %d x %d chunks (%,d bytes)%n", n, n, len);
            System.out.printf("    rust hash     : 0x%016x%n", rustHash);
            System.out.printf("    java hash     : 0x%016x%n", javaHash);
            check(rustHash == javaHash,
                    "boundary corrupted the data: rust=0x" + Long.toHexString(rustHash)
                            + " java=0x" + Long.toHexString(javaHash));
            System.out.println("    PASS: 1.5 MB crossed the FFI boundary byte-for-byte intact");

            // Sanity: the terrain is not degenerate.
            long solid = 0;
            for (long i = 0; i < len; i += 997) {
                if (buf.get(ValueLayout.JAVA_BYTE, i) != 0) solid++;
            }
            double solidFrac = (double) solid / (len / 997.0);
            check(solidFrac > 0.05 && solidFrac < 0.95,
                    "terrain looks degenerate, solid fraction = " + solidFrac);
            System.out.printf(Locale.ROOT, "    solid fraction: %.1f%% (plausible)%n",
                    solidFrac * 100);

            // --- 4. determinism -------------------------------------------
            System.out.println();
            System.out.println("[4] determinism");
            MemorySegment hashSlot2 = arena.allocate(ValueLayout.JAVA_LONG);
            rc = (int) h.fingerprint.invokeExact(world, 0, 0, n, hashSlot2);
            check(rc == OK, "second fingerprint failed");
            check(hashSlot2.get(ValueLayout.JAVA_LONG, 0) == rustHash,
                    "repeated generation was not deterministic");
            System.out.println("    PASS: regenerating the same region gives the same hash");

            // --- 5. parallel speedup --------------------------------------
            System.out.println();
            System.out.println("[5] parallel vs serial fill");
            int bn = 8;
            long blen = (long) bn * bn * BLOCKS_PER_CHUNK;
            MemorySegment big = arena.allocate(blen);

            // Warm up both paths so JIT/native caches are not part of the number.
            for (int i = 0; i < 2; i++) {
                rc = (int) h.fillRegionSerial.invokeExact(world, 0, 0, bn, big, blen);
                check(rc == OK, "warmup serial fill failed rc=" + rc);
                rc = (int) h.fillRegion.invokeExact(world, 0, 0, bn, big, blen);
                check(rc == OK, "warmup parallel fill failed rc=" + rc);
            }

            long serialBest = Long.MAX_VALUE;
            for (int i = 0; i < 3; i++) {
                long t0 = System.nanoTime();
                rc = (int) h.fillRegionSerial.invokeExact(world, 0, 0, bn, big, blen);
                check(rc == OK, "serial fill failed rc=" + rc);
                serialBest = Math.min(serialBest, System.nanoTime() - t0);
            }
            long parBest = Long.MAX_VALUE;
            for (int i = 0; i < 3; i++) {
                long t0 = System.nanoTime();
                rc = (int) h.fillRegion.invokeExact(world, 0, 0, bn, big, blen);
                check(rc == OK, "parallel fill failed rc=" + rc);
                parBest = Math.min(parBest, System.nanoTime() - t0);
            }

            int chunks = bn * bn;
            double serialCps = chunks / (serialBest / 1e9);
            double parCps = chunks / (parBest / 1e9);
            System.out.printf("    region      : %d chunks (%,d blocks)%n",
                    chunks, blen);
            System.out.printf("    serial      : %s  -> %,.1f chunks/s%n",
                    fmt(serialBest), serialCps);
            System.out.printf("    parallel    : %s  -> %,.1f chunks/s%n",
                    fmt(parBest), parCps);
            System.out.printf(Locale.ROOT, "    speedup     : %.2fx on %d cpus%n",
                    (double) serialBest / parBest, Runtime.getRuntime().availableProcessors());
            check(parBest < serialBest,
                    "parallel fill was not faster than serial (threading not working?)");

            // --- 6. raw noise throughput ---------------------------------
            System.out.println();
            System.out.println("[6] raw noise throughput");
            MemorySegment sumSlot = arena.allocate(ValueLayout.JAVA_DOUBLE);
            long samples = 20_000_000L;
            rc = (int) h.noiseSamples.invokeExact(world, 1_000_000L, sumSlot); // warm up
            long t0 = System.nanoTime();
            rc = (int) h.noiseSamples.invokeExact(world, samples, sumSlot);
            long noiseNanos = System.nanoTime() - t0;
            check(rc == OK, "noise sampling failed rc=" + rc);
            double perSec = samples / (noiseNanos / 1e9);
            System.out.printf(Locale.ROOT, "    samples     : %,d in %s%n",
                    samples, fmt(noiseNanos));
            System.out.printf(Locale.ROOT, "    throughput  : %,.1f M samples/s%n", perSec / 1e6);
            System.out.printf(Locale.ROOT, "    checksum    : %.12f (returned, not optimised away)%n",
                    sumSlot.get(ValueLayout.JAVA_DOUBLE, 0));
            check(Double.isFinite(sumSlot.get(ValueLayout.JAVA_DOUBLE, 0)),
                    "noise sum was not finite");

            // --- 7. error handling ---------------------------------------
            System.out.println();
            System.out.println("[7] misuse is rejected, not ignored");
            rc = (int) h.fillRegion.invokeExact(world, 0, 0, 4, buf, 16L); // too small
            check(rc == ERR_LEN, "expected ERR_LEN for a short buffer, got " + rc);
            System.out.println("    PASS: short buffer -> rc=" + rc
                    + " (" + h.errorMessage(arena) + ")");

            rc = (int) h.fillRegion.invokeExact(MemorySegment.NULL, 0, 0, 1, buf, len);
            check(rc == ERR_NULL, "expected ERR_NULL for a null handle, got " + rc);
            System.out.println("    PASS: null handle  -> rc=" + rc);

            h.worldDestroy.invokeExact(world);
        }

        System.out.println();
        System.out.println("=== ALL CHECKS PASSED ===");
    }
}
