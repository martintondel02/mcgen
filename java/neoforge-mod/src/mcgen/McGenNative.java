package mcgen;

import java.nio.ByteBuffer;

/**
 * Thin Java view over the Rust library, via the JNI shim in
 * {@code native/jni_shim.c}.
 *
 * <p>Every method maps 1:1 onto a C entry point. Return codes are the constants
 * in {@link McGenMod}; {@link #lastError()} carries the message for the most
 * recent failure.
 *
 * <p>Buffers are direct {@link ByteBuffer}s so the native side can write into
 * them without a copy — terrain for a 16×16-chunk region is 25 MB, and copying
 * that twice per call would undo the point of the exercise.
 */
public final class McGenNative {

    private McGenNative() {}

    /** ABI version of the loaded library. Must equal {@link McGenMod#ABI_VERSION}. */
    public static native int abiVersion();

    /** Create a world for {@code seed}. Returns 0 on failure (see {@link #lastError()}). */
    public static native long create(long seed);

    /** Release a handle from {@link #create}. Safe to call with 0. */
    public static native void destroy(long handle);

    /** Fill an n×n chunk region, multithreaded. */
    public static native int fillRegion(long handle, int chunkX, int chunkZ, int n, ByteBuffer out);

    /** Fill an n×n chunk region on one thread, for speedup measurements. */
    public static native int fillRegionSerial(long handle, int chunkX, int chunkZ, int n, ByteBuffer out);

    /** Hash the generated region; {@code out[0]} receives the result. */
    public static native int fingerprint(long handle, int chunkX, int chunkZ, int n, long[] out);

    /** Sample noise {@code count} times; {@code out[0]} receives a checksum. */
    public static native int noiseSamples(long handle, long count, double[] out);

    /** Message for the most recent failure. */
    public static native String lastError();

    // ---- used by the /mcgen commands -------------------------------------

    /** Fill exactly one chunk; {@code out} must hold {@link McGenMod#BLOCKS_PER_CHUNK} bytes. */
    public static native int fillChunk(long handle, int chunkX, int chunkZ, ByteBuffer out);

    /** How many noises the loaded graph references; {@code out[0]} receives the count. */
    public static native int noiseCount(long handle, int[] out);

    /**
     * Copy noise {@code index}'s registry name into {@code out} as UTF-8.
     * {@code outLen[0]} receives the byte count actually written.
     */
    public static native int noiseName(long handle, int index, byte[] out, int[] outLen);

    /** The seed this world was built for; {@code out[0]} receives it. */
    public static native int seed(long handle, long[] out);

    /** Evaluate the density graph at one world position; {@code out[0]} receives it. */
    public static native int density(long handle, double x, double y, double z, double[] out);
}
