/*
 * JNI shim between the JVM and the Rust mcgen library.
 *
 * Why JNI rather than Java 21's FFM API?
 *
 * The foreign-function API is a *preview* feature in Java 21 (JEP 442) and only
 * became final in Java 22 (JEP 454). NeoForge 1.21.1 targets Java 21, so a mod
 * using FFM would force every server admin to add `--enable-preview` to their
 * launch arguments. JNI has no such requirement, so this is what ships inside
 * the server; the FFM path is still used by the standalone harness where we
 * control the flags.
 *
 * The shim is intentionally trivial: it converts JNI types to the C ABI and
 * nothing else. All real logic stays in Rust.
 */

#include <jni.h>
#include <stdint.h>
#include <stddef.h>

/* ---- Rust C ABI (see rust/mcgen-ffi/src/lib.rs) ------------------------- */
extern uint32_t mcgen_abi_version(void);
extern int32_t mcgen_world_create(uint64_t seed, void **out);
extern void mcgen_world_destroy(void *world);
extern int32_t mcgen_world_fill_region(const void *world, int32_t chunk_x,
                                       int32_t chunk_z, int32_t n,
                                       uint8_t *out, size_t out_len);
extern int32_t mcgen_world_fill_region_serial(const void *world, int32_t chunk_x,
                                              int32_t chunk_z, int32_t n,
                                              uint8_t *out, size_t out_len);
extern int32_t mcgen_world_fingerprint(const void *world, int32_t chunk_x,
                                       int32_t chunk_z, int32_t n, uint64_t *out);
extern int32_t mcgen_world_noise_samples(const void *world, uint64_t count,
                                         double *out_sum);
extern int32_t mcgen_last_error(char *out, size_t len);
extern int32_t mcgen_world_noise_count(const void *world, uint32_t *out);
extern int32_t mcgen_world_noise_name(const void *world, uint32_t index,
                                      uint8_t *buf, size_t buf_len,
                                      size_t *out_len);
extern int32_t mcgen_world_seed(const void *world, uint64_t *out);
extern int32_t mcgen_world_density(const void *world, double x, double y,
                                   double z, double *out);
extern int32_t mcgen_world_fill_chunk(const void *world, int32_t chunk_x,
                                      int32_t chunk_z, uint8_t *out);

/* Error codes mirrored from the Rust side. */
#define MCGEN_ERR_NULL 1
#define MCGEN_ERR_LEN 2

JNIEXPORT jint JNICALL
Java_mcgen_McGenNative_abiVersion(JNIEnv *env, jclass cls) {
    (void)env;
    (void)cls;
    return (jint)mcgen_abi_version();
}

JNIEXPORT jlong JNICALL
Java_mcgen_McGenNative_create(JNIEnv *env, jclass cls, jlong seed) {
    (void)env;
    (void)cls;
    void *world = NULL;
    if (mcgen_world_create((uint64_t)seed, &world) != 0) {
        return 0; /* 0 becomes "null handle" on the Java side */
    }
    return (jlong)(uintptr_t)world;
}

JNIEXPORT void JNICALL
Java_mcgen_McGenNative_destroy(JNIEnv *env, jclass cls, jlong handle) {
    (void)env;
    (void)cls;
    mcgen_world_destroy((void *)(uintptr_t)handle);
}

/*
 * Zero-copy: the Java side passes a direct ByteBuffer, so we hand Rust the
 * buffer address directly rather than copying a megabyte through the JNI
 * array APIs.
 */
JNIEXPORT jint JNICALL
Java_mcgen_McGenNative_fillRegion(JNIEnv *env, jclass cls, jlong handle,
                                   jint chunk_x, jint chunk_z, jint n,
                                   jobject buffer) {
    (void)cls;
    if (handle == 0) return MCGEN_ERR_NULL;

    void *addr = (*env)->GetDirectBufferAddress(env, buffer);
    jlong cap = (*env)->GetDirectBufferCapacity(env, buffer);
    if (addr == NULL || cap < 0) return MCGEN_ERR_LEN;

    return (jint)mcgen_world_fill_region((const void *)(uintptr_t)handle,
                                         chunk_x, chunk_z, n,
                                         (uint8_t *)addr, (size_t)cap);
}

JNIEXPORT jint JNICALL
Java_mcgen_McGenNative_fillRegionSerial(JNIEnv *env, jclass cls, jlong handle,
                                         jint chunk_x, jint chunk_z, jint n,
                                         jobject buffer) {
    (void)cls;
    if (handle == 0) return MCGEN_ERR_NULL;

    void *addr = (*env)->GetDirectBufferAddress(env, buffer);
    jlong cap = (*env)->GetDirectBufferCapacity(env, buffer);
    if (addr == NULL || cap < 0) return MCGEN_ERR_LEN;

    return (jint)mcgen_world_fill_region_serial((const void *)(uintptr_t)handle,
                                                chunk_x, chunk_z, n,
                                                (uint8_t *)addr, (size_t)cap);
}

JNIEXPORT jint JNICALL
Java_mcgen_McGenNative_fingerprint(JNIEnv *env, jclass cls, jlong handle,
                                    jint chunk_x, jint chunk_z, jint n,
                                    jlongArray out) {
    (void)cls;
    if (handle == 0) return MCGEN_ERR_NULL;

    uint64_t hash = 0;
    int32_t rc = mcgen_world_fingerprint((const void *)(uintptr_t)handle,
                                         chunk_x, chunk_z, n, &hash);
    if (rc != 0) return (jint)rc;

    (*env)->SetLongArrayRegion(env, out, 0, 1, (const jlong *)&hash);
    return 0;
}

JNIEXPORT jint JNICALL
Java_mcgen_McGenNative_noiseSamples(JNIEnv *env, jclass cls, jlong handle,
                                     jlong count, jdoubleArray out) {
    (void)cls;
    if (handle == 0) return MCGEN_ERR_NULL;

    double sum = 0.0;
    int32_t rc = mcgen_world_noise_samples((const void *)(uintptr_t)handle,
                                           (uint64_t)count, &sum);
    if (rc != 0) return (jint)rc;

    (*env)->SetDoubleArrayRegion(env, out, 0, 1, (const jdouble *)&sum);
    return 0;
}

JNIEXPORT jstring JNICALL
Java_mcgen_McGenNative_lastError(JNIEnv *env, jclass cls) {
    (void)cls;
    char buf[512];
    int32_t n = mcgen_last_error(buf, sizeof(buf));
    if (n <= 0) return (*env)->NewStringUTF(env, "(no error recorded)");
    return (*env)->NewStringUTF(env, buf);
}

/* ---- added for the /mcgen command suite -------------------------------- */

JNIEXPORT jint JNICALL
Java_mcgen_McGenNative_noiseCount(JNIEnv *env, jclass cls, jlong handle,
                                   jintArray out) {
    (void)cls;
    if (handle == 0) return MCGEN_ERR_NULL;

    uint32_t n = 0;
    int32_t rc = mcgen_world_noise_count((const void *)(uintptr_t)handle, &n);
    if (rc != 0) return (jint)rc;

    jint v = (jint)n;
    (*env)->SetIntArrayRegion(env, out, 0, 1, &v);
    return 0;
}

/*
 * Copies the name into the caller's byte array and sets element 0 of `out` to
 * the length. Returning a jstring would be simpler, but the shim deliberately
 * stays free of string encoding decisions: the bytes are UTF-8 and Java decodes
 * them, which is one fewer place for a mismatch.
 */
JNIEXPORT jint JNICALL
Java_mcgen_McGenNative_noiseName(JNIEnv *env, jclass cls, jlong handle,
                                  jint index, jbyteArray out, jintArray outLen) {
    (void)cls;
    if (handle == 0) return MCGEN_ERR_NULL;

    jsize cap = (*env)->GetArrayLength(env, out);
    if (cap <= 0) return MCGEN_ERR_LEN;

    uint8_t *buf = (uint8_t *)(*env)->GetByteArrayElements(env, out, NULL);
    if (buf == NULL) return MCGEN_ERR_NULL;

    size_t written = 0;
    int32_t rc = mcgen_world_noise_name((const void *)(uintptr_t)handle,
                                        (uint32_t)index, buf, (size_t)cap,
                                        &written);
    (*env)->ReleaseByteArrayElements(env, out, (jbyte *)buf,
                                     rc == 0 ? 0 : JNI_ABORT);
    if (rc != 0) return (jint)rc;

    jint len = (jint)written;
    (*env)->SetIntArrayRegion(env, outLen, 0, 1, &len);
    return 0;
}

JNIEXPORT jint JNICALL
Java_mcgen_McGenNative_seed(JNIEnv *env, jclass cls, jlong handle,
                             jlongArray out) {
    (void)cls;
    if (handle == 0) return MCGEN_ERR_NULL;

    uint64_t seed = 0;
    int32_t rc = mcgen_world_seed((const void *)(uintptr_t)handle, &seed);
    if (rc != 0) return (jint)rc;

    jlong v = (jlong)seed;
    (*env)->SetLongArrayRegion(env, out, 0, 1, &v);
    return 0;
}

JNIEXPORT jint JNICALL
Java_mcgen_McGenNative_density(JNIEnv *env, jclass cls, jlong handle,
                                jdouble x, jdouble y, jdouble z,
                                jdoubleArray out) {
    (void)cls;
    if (handle == 0) return MCGEN_ERR_NULL;

    double d = 0.0;
    int32_t rc = mcgen_world_density((const void *)(uintptr_t)handle, x, y, z, &d);
    if (rc != 0) return (jint)rc;

    (*env)->SetDoubleArrayRegion(env, out, 0, 1, (const jdouble *)&d);
    return 0;
}

JNIEXPORT jint JNICALL
Java_mcgen_McGenNative_fillChunk(JNIEnv *env, jclass cls, jlong handle,
                                  jint chunk_x, jint chunk_z, jobject buffer) {
    (void)cls;
    if (handle == 0) return MCGEN_ERR_NULL;

    void *addr = (*env)->GetDirectBufferAddress(env, buffer);
    jlong cap = (*env)->GetDirectBufferCapacity(env, buffer);
    if (addr == NULL || cap < 0) return MCGEN_ERR_LEN;

    return (jint)mcgen_world_fill_chunk((const void *)(uintptr_t)handle,
                                        chunk_x, chunk_z, (uint8_t *)addr);
}
