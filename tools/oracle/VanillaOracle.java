import net.minecraft.util.RandomSource;
import net.minecraft.world.level.levelgen.LegacyRandomSource;
import net.minecraft.world.level.levelgen.XoroshiroRandomSource;
import net.minecraft.world.level.levelgen.synth.ImprovedNoise;
import net.minecraft.world.level.levelgen.synth.NormalNoise;

/**
 * Vanilla parity oracle.
 *
 * <p>Runs against the <b>real Minecraft 1.21.1 classes</b> shipped in the
 * NeoForge server install, so the numbers it prints are ground truth rather
 * than a reimplementation of ground truth. The Rust side must reproduce them
 * bit for bit.
 *
 * <p>Output is one record per line, deliberately trivial to diff:
 *
 * <pre>{@code
 * <kind> <seed> <index> <16-hex-digit raw double bits>
 * }</pre>
 *
 * <p>Doubles are printed as raw bit patterns ({@code Double.doubleToRawLongBits})
 * because the whole point is bit-exactness: a difference in the last mantissa
 * bit is a real difference, and {@code %f} would hide it. The same trick lets us
 * distinguish {@code -0.0} from {@code 0.0} and catch NaN payloads.
 *
 * <p>Usage: {@code java VanillaOracle <kind> [more kinds...]}
 */
public final class VanillaOracle {

    /**
     * Results go here, not to {@code System.out}.
     *
     * <p>Minecraft installs a {@code LoggedPrintStream} that prefixes every line
     * with a timestamp and redirects it through log4j. Writing to our own stream
     * keeps the data clean, so callers never have to strip log noise.
     */
    static java.io.PrintStream OUT = System.out;

    /** Seeds used on both sides. Chosen to cover zero, small, and full-width values. */
    static final long[] SEEDS = {0L, 1L, 42L, 123456789L, 1779920288596261407L, -1L};

    static final int SAMPLES = 24;

    public static void main(String[] args) throws Exception {
        String outPath = System.getProperty("mcgen.out");
        if (outPath != null) {
            OUT = new java.io.PrintStream(
                    new java.io.FileOutputStream(outPath), false, "UTF-8");
        }

        java.util.List<String> kinds = args.length > 0
                ? java.util.List.of(args)
                : java.util.List.of("improved_noise", "xoroshiro", "normal_noise", "hash", "density", "density_noise");

        for (int ki = 0; ki < kinds.size(); ki++) {
            String kind = kinds.get(ki);
            switch (kind) {
                case "improved_noise" -> improvedNoise();
                case "xoroshiro" -> xoroshiro();
                case "normal_noise" -> normalNoise();
                case "hash" -> hashStrings();
                case "splitter" -> splitter();
                case "density" -> density();
                case "density_noise" -> densityNoise();
                case "density_json" -> {
                    // Consumes the following argument as its path.
                    densityJson(kinds.get(ki + 1));
                    ki++;
                }
                case "density_encode" -> densityEncode();
                case "blended" -> blended();
                case "math" -> math();
                case "density_dense" -> {
                    // Consumes the following argument as its path.
                    densityDense(kinds.get(ki + 1));
                    ki++;
                }
                default -> throw new IllegalArgumentException("unknown kind: " + kind);
            }
        }
    }

    /** Deterministic sample coordinates. Identical arithmetic must be used in Rust. */
    static double sampleX(int i) {
        return (i - 12) * 0.37;
    }

    static double sampleY(int i) {
        return i * 0.125 + 0.5;
    }

    static double sampleZ(int i) {
        return (12 - i) * 0.53;
    }

    // ---- density-function layer -------------------------------------------
    //
    // Vanilla density functions are evaluated at *integer block coordinates*
    // (FunctionContext.blockX/Y/Z), not at block centres. Sampling at +0.5 would
    // be a silent, systematic offset, so these are integers on both sides.

    static int blockX(int i) {
        return (i - 12) * 7 + 3;
    }

    static int blockY(int i) {
        return i * 5 - 40;
    }

    static int blockZ(int i) {
        return (12 - i) * 11 - 3;
    }

    /**
     * Every density-function node type that does not need a registry, built with
     * vanilla's own factories and evaluated with vanilla's own code. This pins
     * down the arithmetic, the mapped unary operators, the Y gradient, and the
     * caching wrappers, plus the coordinate convention.
     */
    static void density() {
        // Touching vanilla registries from a standalone program requires the same
        // two-step bootstrap the server does: detect the game version, then
        // bootstrap the registries. Without the first step, bootstrapping dies in
        // SharedConstants with "Game version not set".
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();

        record Case(String name, net.minecraft.world.level.levelgen.DensityFunction fn) {}

        var c = net.minecraft.world.level.levelgen.DensityFunctions.constant(-0.5);
        var zero = net.minecraft.world.level.levelgen.DensityFunctions.constant(0.0);

        java.util.List<Case> cases = java.util.List.of(
                new Case("constant", net.minecraft.world.level.levelgen.DensityFunctions.constant(1.5)),
                new Case("y_clamped_gradient",
                        net.minecraft.world.level.levelgen.DensityFunctions.yClampedGradient(-64, 320, 2.0, -3.0)),
                new Case("add", net.minecraft.world.level.levelgen.DensityFunctions.add(
                        net.minecraft.world.level.levelgen.DensityFunctions.constant(1.0),
                        net.minecraft.world.level.levelgen.DensityFunctions.constant(2.0))),
                new Case("mul", net.minecraft.world.level.levelgen.DensityFunctions.mul(
                        net.minecraft.world.level.levelgen.DensityFunctions.constant(3.0),
                        net.minecraft.world.level.levelgen.DensityFunctions.constant(-4.0))),
                new Case("min", net.minecraft.world.level.levelgen.DensityFunctions.min(
                        net.minecraft.world.level.levelgen.DensityFunctions.constant(1.0),
                        net.minecraft.world.level.levelgen.DensityFunctions.constant(2.0))),
                new Case("max", net.minecraft.world.level.levelgen.DensityFunctions.max(
                        net.minecraft.world.level.levelgen.DensityFunctions.constant(1.0),
                        net.minecraft.world.level.levelgen.DensityFunctions.constant(2.0))),
                new Case("abs", c.abs()),
                new Case("square", c.square()),
                new Case("cube", c.cube()),
                new Case("half_negative", c.halfNegative()),
                new Case("quarter_negative", c.quarterNegative()),
                new Case("squeeze", c.squeeze()),
                new Case("abs_positive", zero.abs()),
                new Case("clamp", net.minecraft.world.level.levelgen.DensityFunctions.constant(5.0).clamp(-1.0, 1.0)),
                new Case("flat_cache", net.minecraft.world.level.levelgen.DensityFunctions.flatCache(
                        net.minecraft.world.level.levelgen.DensityFunctions.constant(2.5))),
                new Case("cache_once", net.minecraft.world.level.levelgen.DensityFunctions.cacheOnce(
                        net.minecraft.world.level.levelgen.DensityFunctions.constant(2.5))),
                new Case("cache_2d", net.minecraft.world.level.levelgen.DensityFunctions.cache2d(
                        net.minecraft.world.level.levelgen.DensityFunctions.constant(2.5))),
                new Case("nested_add_mul", net.minecraft.world.level.levelgen.DensityFunctions.add(
                        net.minecraft.world.level.levelgen.DensityFunctions.yClampedGradient(-64, 320, 2.0, -3.0),
                        net.minecraft.world.level.levelgen.DensityFunctions.mul(
                                net.minecraft.world.level.levelgen.DensityFunctions.constant(0.5),
                                c.squeeze()))));

        for (Case k : cases) {
            for (int i = 0; i < SAMPLES; i++) {
                final int bx = blockX(i);
                final int by = blockY(i);
                final int bz = blockZ(i);
                var ctx = new net.minecraft.world.level.levelgen.DensityFunction.FunctionContext() {
                    @Override public int blockX() { return bx; }
                    @Override public int blockY() { return by; }
                    @Override public int blockZ() { return bz; }
                };
                emit(k.name(), 0L, i, k.fn().compute(ctx));
            }
        }
    }

    /**
     * Density nodes that carry noise: `noise`, `shift`, `shift_a`, `shift_b`, and
     * `shifted_noise`.
     *
     * <p>These need an initialised `NormalNoise` on each `NoiseHolder`.
     * `RandomState` normally does that via a `Visitor`; we do the same, handing
     * each holder the exact instance the Rust side builds.
     */
    static void densityNoise() {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();

        final long seed = 4242L;
        var params = new net.minecraft.world.level.levelgen.synth.NormalNoise.NoiseParameters(
                -7, 1.0, 1.0, 1.0, 1.0);
        var holder = net.minecraft.core.Holder.direct(params);

        // Must match Rust: XoroshiroRandomSource::new(4242) + new_modern(-7, [1,1,1,1]).
        var rs = new XoroshiroRandomSource(seed);
        final NormalNoise nn = NormalNoise.create(rs, -7, 1.0, 1.0, 1.0, 1.0);

        net.minecraft.world.level.levelgen.DensityFunction.Visitor init =
                new net.minecraft.world.level.levelgen.DensityFunction.Visitor() {
                    @Override
                    public net.minecraft.world.level.levelgen.DensityFunction apply(
                            net.minecraft.world.level.levelgen.DensityFunction f) {
                        return f;
                    }

                    @Override
                    public net.minecraft.world.level.levelgen.DensityFunction.NoiseHolder visitNoise(
                            net.minecraft.world.level.levelgen.DensityFunction.NoiseHolder h) {
                        return new net.minecraft.world.level.levelgen.DensityFunction.NoiseHolder(
                                h.noiseData(), nn);
                    }
                };

        record Case(String name, net.minecraft.world.level.levelgen.DensityFunction fn) {}

        java.util.List<Case> cases = java.util.List.of(
                new Case("noise_default",
                        net.minecraft.world.level.levelgen.DensityFunctions.noise(holder).mapAll(init)),
                new Case("noise_scaled",
                        net.minecraft.world.level.levelgen.DensityFunctions.noise(holder, 0.25, 0.0).mapAll(init)),
                new Case("noise_xz_y",
                        net.minecraft.world.level.levelgen.DensityFunctions.noise(holder, 0.02, 0.06).mapAll(init)),
                new Case("shift_a",
                        net.minecraft.world.level.levelgen.DensityFunctions.shiftA(holder).mapAll(init)),
                new Case("shift_b",
                        net.minecraft.world.level.levelgen.DensityFunctions.shiftB(holder).mapAll(init)),
                new Case("shift",
                        net.minecraft.world.level.levelgen.DensityFunctions.shift(holder).mapAll(init)),
                new Case("shifted_noise_2d",
                        net.minecraft.world.level.levelgen.DensityFunctions.shiftedNoise2d(
                                net.minecraft.world.level.levelgen.DensityFunctions.shiftA(holder),
                                net.minecraft.world.level.levelgen.DensityFunctions.shiftB(holder),
                                0.25, holder).mapAll(init)));

        for (Case k : cases) {
            for (int i = 0; i < SAMPLES; i++) {
                final int bx = blockX(i);
                final int by = blockY(i);
                final int bz = blockZ(i);
                var ctx = new net.minecraft.world.level.levelgen.DensityFunction.FunctionContext() {
                    @Override public int blockX() { return bx; }
                    @Override public int blockY() { return by; }
                    @Override public int blockZ() { return bz; }
                };
                emit(k.name(), 0L, i, k.fn().compute(ctx));
            }
        }
    }

    /**
     * Print vanilla's own JSON encoding of every registry-free node type.
     *
     * <p>This is the authoritative schema: rather than guessing field names (the
     * first guess, `value` for `constant`, was wrong — vanilla uses `argument`),
     * we let vanilla serialise its own functions and use that output as the
     * shared test corpus.
     */
    static void densityEncode() {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();

        var D = net.minecraft.world.level.levelgen.DensityFunctions.class;
        var c = net.minecraft.world.level.levelgen.DensityFunctions.constant(-0.5);

        var cases = new java.util.LinkedHashMap<String, net.minecraft.world.level.levelgen.DensityFunction>();
        cases.put("constant", net.minecraft.world.level.levelgen.DensityFunctions.constant(1.5));
        cases.put("y_clamped_gradient", net.minecraft.world.level.levelgen.DensityFunctions.yClampedGradient(-64, 320, 2.0, -3.0));
        cases.put("add", net.minecraft.world.level.levelgen.DensityFunctions.add(
                net.minecraft.world.level.levelgen.DensityFunctions.constant(1.0),
                net.minecraft.world.level.levelgen.DensityFunctions.constant(2.0)));
        cases.put("mul", net.minecraft.world.level.levelgen.DensityFunctions.mul(
                net.minecraft.world.level.levelgen.DensityFunctions.constant(3.0),
                net.minecraft.world.level.levelgen.DensityFunctions.constant(-4.0)));
        cases.put("min", net.minecraft.world.level.levelgen.DensityFunctions.min(
                net.minecraft.world.level.levelgen.DensityFunctions.constant(1.0),
                net.minecraft.world.level.levelgen.DensityFunctions.constant(2.0)));
        cases.put("max", net.minecraft.world.level.levelgen.DensityFunctions.max(
                net.minecraft.world.level.levelgen.DensityFunctions.constant(1.0),
                net.minecraft.world.level.levelgen.DensityFunctions.constant(2.0)));
        cases.put("abs", c.abs());
        cases.put("square", c.square());
        cases.put("cube", c.cube());
        cases.put("half_negative", c.halfNegative());
        cases.put("quarter_negative", c.quarterNegative());
        cases.put("squeeze", c.squeeze());
        cases.put("clamp", net.minecraft.world.level.levelgen.DensityFunctions.constant(5.0).clamp(-1.0, 1.0));
        cases.put("flat_cache", net.minecraft.world.level.levelgen.DensityFunctions.flatCache(
                net.minecraft.world.level.levelgen.DensityFunctions.constant(2.5)));
        cases.put("cache_once", net.minecraft.world.level.levelgen.DensityFunctions.cacheOnce(
                net.minecraft.world.level.levelgen.DensityFunctions.constant(2.5)));
        cases.put("cache_2d", net.minecraft.world.level.levelgen.DensityFunctions.cache2d(
                net.minecraft.world.level.levelgen.DensityFunctions.constant(2.5)));
        // --- splines -------------------------------------------------------
        // The coordinate is a density function, so these vary with block Y and
        // actually exercise the interpolation rather than returning a constant.
        var ramp = net.minecraft.core.Holder.<net.minecraft.world.level.levelgen.DensityFunction>direct(
                net.minecraft.world.level.levelgen.DensityFunctions.yClampedGradient(-64, 320, 0.0, 1.0));

        cases.put("spline_two_point",
                net.minecraft.world.level.levelgen.DensityFunctions.spline(
                        net.minecraft.util.CubicSpline.builder(
                                        new net.minecraft.world.level.levelgen.DensityFunctions.Spline.Coordinate(ramp))
                                .addPoint(0.0f, 0.0f, 0.0f)
                                .addPoint(1.0f, 10.0f, 0.0f)
                                .build()));

        cases.put("spline_with_derivatives",
                net.minecraft.world.level.levelgen.DensityFunctions.spline(
                        net.minecraft.util.CubicSpline.builder(
                                        new net.minecraft.world.level.levelgen.DensityFunctions.Spline.Coordinate(ramp))
                                .addPoint(0.0f, 0.0f, 2.0f)
                                .addPoint(1.0f, 10.0f, -1.0f)
                                .build()));

        cases.put("spline_three_point",
                net.minecraft.world.level.levelgen.DensityFunctions.spline(
                        net.minecraft.util.CubicSpline.builder(
                                        new net.minecraft.world.level.levelgen.DensityFunctions.Spline.Coordinate(ramp))
                                .addPoint(0.0f, 0.0f, 0.0f)
                                .addPoint(0.5f, 10.0f, 1.0f)
                                .addPoint(1.0f, -4.0f, 0.0f)
                                .build()));

        // Extrapolation outside the sample range goes through linearExtend.
        cases.put("spline_extrapolate",
                net.minecraft.world.level.levelgen.DensityFunctions.spline(
                        net.minecraft.util.CubicSpline.builder(
                                        new net.minecraft.world.level.levelgen.DensityFunctions.Spline.Coordinate(ramp))
                                .addPoint(0.25f, 1.0f, 3.0f)
                                .addPoint(0.75f, -2.0f, -3.0f)
                                .build()));

        // A spline whose first point's value is itself a spline. Vanilla's codec
        // decides the exact nesting shape, so we let it serialise one.
        var innerSpline = net.minecraft.util.CubicSpline.builder(
                        new net.minecraft.world.level.levelgen.DensityFunctions.Spline.Coordinate(ramp))
                .addPoint(0.0f, -5.0f, 0.0f)
                .addPoint(1.0f, 5.0f, 0.0f)
                .build();
        cases.put("spline_nested",
                net.minecraft.world.level.levelgen.DensityFunctions.spline(
                        net.minecraft.util.CubicSpline.builder(
                                        new net.minecraft.world.level.levelgen.DensityFunctions.Spline.Coordinate(ramp))
                                .addPoint(0.0f, innerSpline)
                                .addPoint(1.0f, 1.0f)
                                .build()));

        cases.put("range_choice", net.minecraft.world.level.levelgen.DensityFunctions.rangeChoice(
                net.minecraft.world.level.levelgen.DensityFunctions.constant(0.5), -1.0, 1.0,
                net.minecraft.world.level.levelgen.DensityFunctions.constant(10.0),
                net.minecraft.world.level.levelgen.DensityFunctions.constant(20.0)));

        for (var e : cases.entrySet()) {
            var encoded = net.minecraft.world.level.levelgen.DensityFunctions.DIRECT_CODEC
                    .encodeStart(com.mojang.serialization.JsonOps.INSTANCE, e.getValue());
            String json = encoded.getOrThrow(msg -> new IllegalStateException(e.getKey() + ": " + msg))
                    .toString();
            OUT.println(e.getKey() + "\t" + json);
        }
    }

    /**
     * Initialise every noise holder with a `NormalNoise` built from seed 0.
     *
     * <p>Density graphs do not carry their noise: the holders stay inert until a
     * visitor fills them in, which is what `RandomState` does in game. Without
     * this, every noise-driven node evaluates to zero.
     */
    static net.minecraft.world.level.levelgen.DensityFunction.Visitor seedZeroNoiseVisitor() {
        return new net.minecraft.world.level.levelgen.DensityFunction.Visitor() {
            @Override
            public net.minecraft.world.level.levelgen.DensityFunction apply(
                    net.minecraft.world.level.levelgen.DensityFunction f) {
                return f;
            }

            @Override
            public net.minecraft.world.level.levelgen.DensityFunction.NoiseHolder visitNoise(
                    net.minecraft.world.level.levelgen.DensityFunction.NoiseHolder h) {
                NormalNoise nn = NormalNoise.create(
                        new XoroshiroRandomSource(0L), h.noiseData().value());
                return new net.minecraft.world.level.levelgen.DensityFunction.NoiseHolder(
                        h.noiseData(), nn);
            }
        };
    }

    /**
     * JSON-driven density parity.
     *
     * <p>Reads a file of {@code name<TAB>json} lines and evaluates each with
     * vanilla's own {@code DIRECT_CODEC} parser and {@code compute}. Because both
     * sides consume the <i>same JSON</i>, this tests parser and evaluator together,
     * and adding a node type to the harness costs one line.
     *
     * <p>Noise-bearing nodes are excluded: they need a registry to resolve the
     * holder. Those are covered by {@link #densityNoise}.
     */
    static void densityJson(String path) throws java.io.IOException {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();

        var lines = java.nio.file.Files.readAllLines(java.nio.file.Path.of(path));
        for (String line : lines) {
            if (line.isBlank() || line.startsWith("#")) {
                continue;
            }
            int tab = line.indexOf('\t');
            if (tab < 0) {
                throw new IllegalArgumentException("expected name<TAB>json: " + line);
            }
            String name = line.substring(0, tab);
            String json = line.substring(tab + 1);

            var parsed = net.minecraft.world.level.levelgen.DensityFunctions.DIRECT_CODEC
                    .parse(com.mojang.serialization.JsonOps.INSTANCE,
                            com.google.gson.JsonParser.parseString(json));
            var fn = parsed.getOrThrow(msg -> new IllegalStateException(name + ": " + msg));
            // Noise holders start inert; give them the same seed-0 instances the
            // Rust side builds, so noise-bearing graphs are comparable here too.
            fn = fn.mapAll(seedZeroNoiseVisitor());

            for (int i = 0; i < SAMPLES; i++) {
                final int bx = blockX(i);
                final int by = blockY(i);
                final int bz = blockZ(i);
                var ctx = new net.minecraft.world.level.levelgen.DensityFunction.FunctionContext() {
                    @Override public int blockX() { return bx; }
                    @Override public int blockY() { return by; }
                    @Override public int blockZ() { return bz; }
                };
                emit(name, 0L, i, fn.compute(ctx));
            }
        }
    }

    /**
     * Reimplements `BlendedNoise.compute` in Java from the bytecode, and prints
     * both it and the real thing. If the two agree, my reading of the algorithm is
     * right and any Rust mismatch is an implementation bug; if they disagree, the
     * reading itself is wrong.
     */
    static void blended() {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();

        var real = net.minecraft.world.level.levelgen.synth.BlendedNoise.createUnseeded(
                0.25, 0.125, 80.0, 160.0, 8.0);

        // Build our own stacks through the public API so we can drive them.
        var r = new XoroshiroRandomSource(0L);
        var minLimit = net.minecraft.world.level.levelgen.synth.PerlinNoise
                .createLegacyForBlendedNoise(r, java.util.stream.IntStream.rangeClosed(-15, 0));
        var maxLimit = net.minecraft.world.level.levelgen.synth.PerlinNoise
                .createLegacyForBlendedNoise(r, java.util.stream.IntStream.rangeClosed(-15, 0));
        var main = net.minecraft.world.level.levelgen.synth.PerlinNoise
                .createLegacyForBlendedNoise(r, java.util.stream.IntStream.rangeClosed(-7, 0));

        // The private constructor DERIVES these: 684.412 * scale, not scale.
        double xzMultiplier = 684.412 * 0.25, yMultiplier = 684.412 * 0.125;
        double xzFactor = 80.0, yFactor = 160.0;
        double smearScaleMultiplier = 8.0;

        for (int i = 0; i < 4; i++) {
            double x = (i - 12) * 7 + 3;
            double y = i * 5 - 40;
            double z = (12 - i) * 11 - 3;

            double sxFull = x * xzMultiplier, syFull = y * yMultiplier, szFull = z * xzMultiplier;
            double sx = sxFull / xzFactor, sy = syFull / yFactor, sz = szFull / xzFactor;
            double smear = yMultiplier * smearScaleMultiplier;
            double smearFreq = smear / yFactor;

            double mainSum = 0, freq = 1.0;
            var terms = new StringBuilder();
            for (int k = 0; k < 8; k++) {
                var n = main.getOctaveNoise(k);
                if (n != null) {
                    double term = n.noise(
                            net.minecraft.world.level.levelgen.synth.PerlinNoise.wrap(sx * freq),
                            net.minecraft.world.level.levelgen.synth.PerlinNoise.wrap(sy * freq),
                            net.minecraft.world.level.levelgen.synth.PerlinNoise.wrap(sz * freq),
                            smearFreq * freq, sy * freq) / freq;
                    mainSum += term;
                    terms.append(String.format("%.3f ", term));
                }
                freq /= 2.0;
            }
            double d = (mainSum / 10.0 + 1.0) / 2.0;
            System.out.println("  terms[" + i + "] = " + terms + " -> d=" + d);

            // Brute-force the scaling/division combination to find which one
            // reproduces vanilla's implied blend factor.
            for (int scaleArgs = 0; scaleArgs < 2; scaleArgs++) {
                for (int divMode = 0; divMode < 3; divMode++) {
                    double sum = 0; freq = 1.0;
                    for (int k = 0; k < 8; k++) {
                        var n = main.getOctaveNoise(k);
                        if (n != null) {
                            double ax = scaleArgs == 1 ? sx * freq : sx;
                            double ay = scaleArgs == 1 ? sy * freq : sy;
                            double az = scaleArgs == 1 ? sz * freq : sz;
                            double term = n.noise(
                                    net.minecraft.world.level.levelgen.synth.PerlinNoise.wrap(ax),
                                    net.minecraft.world.level.levelgen.synth.PerlinNoise.wrap(ay),
                                    net.minecraft.world.level.levelgen.synth.PerlinNoise.wrap(az),
                                    smearFreq * freq, sy * freq);
                            if (divMode == 0) term = term / freq;
                            else if (divMode == 1) term = term * freq;
                            sum += term;
                        }
                        freq /= 2.0;
                    }
                    System.out.printf("    v[%d] scaleArgs=%d divMode=%d sum=%.10f d=%.10f%n",
                            i, scaleArgs, divMode, sum, (sum / 10.0 + 1.0) / 2.0);
                }
            }

            double minSum = 0, maxSum = 0;
            freq = 1.0;
            for (int k = 0; k < 16; k++) {
                double wx = net.minecraft.world.level.levelgen.synth.PerlinNoise.wrap(sxFull * freq);
                double wy = net.minecraft.world.level.levelgen.synth.PerlinNoise.wrap(syFull * freq);
                double wz = net.minecraft.world.level.levelgen.synth.PerlinNoise.wrap(szFull * freq);
                double wsmear = smear * freq;
                var nMin = minLimit.getOctaveNoise(k);
                if (nMin != null) {
                    minSum += nMin.noise(wx, wy, wz, wsmear, syFull * freq) / freq;
                }
                var nMax = maxLimit.getOctaveNoise(k);
                if (nMax != null) {
                    maxSum += nMax.noise(wx, wy, wz, wsmear, syFull * freq) / freq;
                }
                freq /= 2.0;
            }

            var ctx = new net.minecraft.world.level.levelgen.DensityFunction.FunctionContext() {
                @Override public int blockX() { return (int) x; }
                @Override public int blockY() { return (int) y; }
                @Override public int blockZ() { return (int) z; }
            };
            System.out.printf("blended %d main=%.10f d=%.10f min=%.10f max=%.10f mine=%.12f real=%.12f%n",
                    i, mainSum, d, minSum, maxSum,
                    net.minecraft.util.Mth.clampedLerp(minSum / 512.0, maxSum / 512.0, d) / 128.0,
                    real.compute(ctx));
        }
    }

    /**
     * The interpolation primitives the cell grid is built on, plus vanilla's own
     * `wrap`. Small, but these are exactly what trilinear cell interpolation uses,
     * so they are worth pinning before writing any of it.
     */
    /**
     * `densityJson`, but over a dense grid of chunk-local coordinates.
     *
     * <p>The shared sample coordinates are sparse and never land in the region a chunk
     * generator actually walks. Every node type verified only through them is verified
     * only loosely.
     */
    static void densityDense(String path) throws java.io.IOException {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();

        var text = java.nio.file.Files.readString(java.nio.file.Path.of(path));
        for (String line : text.split("\n")) {
            if (line.isBlank() || line.startsWith("#")) {
                continue;
            }
            int tab = line.indexOf('\t');
            String name = line.substring(0, tab);
            String json = line.substring(tab + 1);

            var parsed = net.minecraft.world.level.levelgen.DensityFunctions.DIRECT_CODEC
                    .parse(com.mojang.serialization.JsonOps.INSTANCE,
                            com.google.gson.JsonParser.parseString(json));
            var fn = parsed.getOrThrow(msg -> new IllegalStateException(name + ": " + msg));
            fn = fn.mapAll(seedZeroNoiseVisitor());

            var contexts = new net.minecraft.world.level.levelgen.DensityFunction.FunctionContext[DENSE_SAMPLES];
            for (int i = 0; i < DENSE_SAMPLES; i++) {
                final int bx = denseX(i);
                final int by = denseY(i);
                final int bz = denseZ(i);
                contexts[i] = new net.minecraft.world.level.levelgen.DensityFunction.FunctionContext() {
                    @Override public int blockX() { return bx; }
                    @Override public int blockY() { return by; }
                    @Override public int blockZ() { return bz; }
                };
            }
            for (int i = 0; i < DENSE_SAMPLES; i++) {
                emit(name, 0L, i, fn.compute(contexts[i]));
            }
        }
    }

    /** 6 x 6 x 10 grid of chunk-local-ish coordinates. */
    static final int DENSE_SAMPLES = 360;
    static final int[] DENSE_X = {0, 1, 2, 3, 7, 15};
    static final int[] DENSE_Y = {-64, -32, 0, 39, 43, 60, 63, 100, 200, 250};
    static final int[] DENSE_Z = {0, 1, 2, 3, 7, 15};

    static int denseX(int i) {
        return DENSE_X[i % 6];
    }

    static int denseZ(int i) {
        return DENSE_Z[(i / 6) % 6];
    }

    static int denseY(int i) {
        return DENSE_Y[i / 36];
    }

    static void math() {
        for (int i = 0; i < 24; i++) {
            double t = i * 0.0416666;
            emit("math_lerp", 0L, i,
                    net.minecraft.util.Mth.lerp(t, -1.5 + i, 2.25 - i));
            emit("math_smoothstep", 0L, i,
                    net.minecraft.util.Mth.smoothstep(t));
            emit("math_clamped_lerp", 0L, i,
                    net.minecraft.util.Mth.clampedLerp(-3.0 + i, 5.0 - i, t));
            emit("math_wrap", 0L, i,
                    net.minecraft.world.level.levelgen.synth.PerlinNoise.wrap(t * 4.0e7 - 2.0e7));

            double ax = i * 0.037, ay = i * 0.061, az = i * 0.017;
            double d000 = 1.0 + i, d100 = -2.0 - i, d010 = 3.5 * i, d110 = 0.25 - i;
            double d001 = -1.0 + 2 * i, d101 = 7.0 - i, d011 = -0.5 * i, d111 = 11.0 + i;
            emit("math_lerp3", 0L, i, net.minecraft.util.Mth.lerp3(
                    ax, ay, az, d000, d100, d010, d110, d001, d101, d011, d111));
            emit("math_lerp2", 0L, i,
                    net.minecraft.util.Mth.lerp2(ax, ay, d000, d100, d010, d110));
        }
    }

    static void emit(String kind, long seed, int index, double value) {
        OUT.printf("%s %d %d %016x%n", kind, seed, index,
                Double.doubleToRawLongBits(value));
    }

    /** The exact octave names a NormalNoise derives, in order. */
    static final String[] HASH_NAMES = {
            "octave_-7", "octave_-6", "octave_-5", "octave_-4",
            "octave_-3", "octave_-2", "octave_-1", "octave_0",
            "TEST STRING", "",
    };

    /**
     * `RandomSupport.seedFromHashOf` in isolation. If the Rust MD5 handling
     * differs, every derived octave stream diverges, so this is the first thing
     * to pin down when normal noise mismatches.
     */
    static void hashStrings() {
        for (int i = 0; i < HASH_NAMES.length; i++) {
            net.minecraft.world.level.levelgen.RandomSupport.Seed128bit s =
                    net.minecraft.world.level.levelgen.RandomSupport.seedFromHashOf(HASH_NAMES[i]);
            OUT.printf("hash %d %016x %016x%n", i, s.seedLo(), s.seedHi());
        }
    }

    /**
     * The splitter itself. `fork()` is the only public window onto it: it is
     * documented as producing a child from the next two draws, which is exactly
     * what `nextSplitter()` does.
     */
    static void splitter() {
        for (long seed : SEEDS) {
            RandomSource rs = new XoroshiroRandomSource(seed);
            for (int i = 0; i < 6; i++) {
                RandomSource child = rs.fork();
                emit("fork_first_long", seed, i, (double) child.nextLong());
            }
        }
    }

    /**
     * Single-octave Perlin against the legacy (java.util.Random) source.
     * This isolates the core noise math: permutation table, coordinate wrap,
     * smoothstep, gradient table, trilinear blend.
     */
    static void improvedNoise() {
        for (long seed : SEEDS) {
            RandomSource rs = new LegacyRandomSource(seed);
            ImprovedNoise noise = new ImprovedNoise(rs);
            for (int i = 0; i < SAMPLES; i++) {
                emit("improved_noise", seed, i, noise.noise(sampleX(i), sampleY(i), sampleZ(i)));
            }
        }
    }

    /**
     * Raw RNG output. nextLong is the primitive; nextDouble is how the noise code
     * actually consumes it, so both are covered.
     */
    static void xoroshiro() {
        for (long seed : SEEDS) {
            RandomSource rs = new XoroshiroRandomSource(seed);
            for (int i = 0; i < SAMPLES; i++) {
                emit("xoroshiro_long", seed, i, (double) rs.nextLong());
            }
        }
        for (long seed : SEEDS) {
            RandomSource rs = new XoroshiroRandomSource(seed);
            for (int i = 0; i < SAMPLES; i++) {
                emit("xoroshiro_double", seed, i, rs.nextDouble());
            }
        }
    }

    /**
     * The normalized, multi-octave noise used by the density functions.
     * This is the modern (splitter) path, so it exercises both the RNG
     * derivation and the octave stacking.
     */
    static void normalNoise() {
        double[][] amplitudeSets = {
                {1.0},
                {1.0, 1.0},
                {1.0, 1.0, 1.0, 1.0},
                {1.5, 0.0, 1.0, 0.0, 0.6},
        };
        for (long seed : SEEDS) {
            for (int a = 0; a < amplitudeSets.length; a++) {
                double[] amps = amplitudeSets[a];
                RandomSource rs = new XoroshiroRandomSource(seed);
                NormalNoise noise = NormalNoise.create(rs, -7, amps);
                for (int i = 0; i < SAMPLES; i++) {
                    emit("normal_noise_a" + a, seed, i,
                            noise.getValue(sampleX(i), sampleY(i), sampleZ(i)));
                }
            }
        }
    }
}
