import com.mojang.serialization.Lifecycle;
import net.minecraft.core.Holder;
import net.minecraft.core.HolderGetter;
import net.minecraft.core.HolderOwner;
import net.minecraft.core.HolderSet;
import net.minecraft.core.Registry;
import net.minecraft.core.registries.Registries;
import net.minecraft.data.worldgen.BootstrapContext;
import net.minecraft.data.worldgen.NoiseData;
import net.minecraft.resources.ResourceKey;
import net.minecraft.tags.TagKey;
import net.minecraft.world.level.levelgen.DensityFunction;
import net.minecraft.world.level.levelgen.DensityFunctions;
import net.minecraft.world.level.levelgen.NoiseGeneratorSettings;
import net.minecraft.world.level.levelgen.NoiseRouter;
import net.minecraft.world.level.levelgen.NoiseRouterData;
import net.minecraft.util.Mth;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.levelgen.Aquifer;
import net.minecraft.world.level.levelgen.NoiseChunk;
import net.minecraft.world.level.levelgen.NoiseSettings;
import net.minecraft.world.level.levelgen.RandomState;
import net.minecraft.world.level.levelgen.blending.Blender;
import net.minecraft.world.level.levelgen.synth.NormalNoise;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;

/**
 * Builds the <b>real</b> overworld density graph by running Minecraft's own data
 * generators, and serialises it to JSON.
 *
 * <p>This is the payoff of the whole oracle approach. Everything verified so far
 * used graphs I built by hand; this one is the actual router that a 1.21.1
 * overworld uses, produced by vanilla's own code. If the Rust side can evaluate
 * this, it can evaluate a real world.
 *
 * <p>Mechanism: {@code BootstrapContext} is a two-method interface, so it can be
 * implemented by a small capture object. Run the three bootstraps in dependency
 * order (noise parameters, then density functions, then generator settings) and
 * pull the OVERWORLD settings out of the capture map.
 */
public final class RouterOracle {

    /** Trivial {@link HolderOwner}; the references never need to serialise. */
    static final class Owner<T> implements HolderOwner<T> { }

    /** A {@link HolderGetter} over a live map, so late registrations are visible. */
    static final class MapGetter<T> implements HolderGetter<T> {
        private final Map<ResourceKey<T>, Holder.Reference<T>> map;

        MapGetter(Map<ResourceKey<T>, Holder.Reference<T>> map) {
            this.map = map;
        }

        @Override
        public Optional<Holder.Reference<T>> get(ResourceKey<T> key) {
            return Optional.ofNullable(map.get(key));
        }

        @Override
        public Optional<HolderSet.Named<T>> get(TagKey<T> key) {
            return Optional.empty();
        }
    }

    /** Captures everything registered through it, and resolves lookups by registry. */
    static final class Capture<T> implements BootstrapContext<T> {
        final Map<ResourceKey<T>, Holder.Reference<T>> map = new LinkedHashMap<>();
        final Map<ResourceKey<?>, HolderGetter<?>> lookups = new HashMap<>();
        private final Owner<T> owner = new Owner<>();

        @Override
        public Holder.Reference<T> register(ResourceKey<T> key, T value, Lifecycle lifecycle) {
            // The holders must be *keyed*: vanilla's codecs refuse to run on an
            // anonymous holder ("Calling .codec() on HolderHolder"), and we need
            // the keys to serialise the graph by noise name.
            //
            // `Reference.bindValue` is protected, so reach it reflectively. This is
            // a test-only tool running on the classpath, so there is no module
            // boundary to fight.
            Holder.Reference<T> ref = Holder.Reference.createStandAlone(owner, key);
            try {
                java.lang.reflect.Method bind =
                        Holder.Reference.class.getDeclaredMethod("bindValue", Object.class);
                bind.setAccessible(true);
                bind.invoke(ref, value);
            } catch (ReflectiveOperationException e) {
                throw new IllegalStateException("could not bind holder " + key, e);
            }
            map.put(key, ref);
            return ref;
        }

        @Override
        @SuppressWarnings("unchecked")
        public <S> HolderGetter<S> lookup(ResourceKey<? extends Registry<? extends S>> registryKey) {
            HolderGetter<?> getter = lookups.get(registryKey);
            if (getter != null) {
                return (HolderGetter<S>) getter;
            }
            // Empty getter rather than null: some bootstraps probe registries
            // they do not actually need.
            return new MapGetter<S>(Map.of());
        }
    }

    /** Print vanilla's own cell-grid parameters, straight from the built settings. */
    static void printNoiseSettings() {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();

        Capture<NormalNoise.NoiseParameters> noise = new Capture<>();
        NoiseData.bootstrap(noise);
        Capture<DensityFunction> density = new Capture<>();
        density.lookups.put(Registries.NOISE, new MapGetter<>(noise.map));
        density.lookups.put(Registries.DENSITY_FUNCTION, new MapGetter<>(density.map));
        NoiseRouterData.bootstrap(density);
        Capture<NoiseGeneratorSettings> settings = new Capture<>();
        settings.lookups.put(Registries.NOISE, new MapGetter<>(noise.map));
        settings.lookups.put(Registries.DENSITY_FUNCTION, new MapGetter<>(density.map));
        NoiseGeneratorSettings.bootstrap(settings);

        var ns = settings.map.get(NoiseGeneratorSettings.OVERWORLD).value().noiseSettings();
        OUT.printf("randomSource=%s%n", settings.map.get(NoiseGeneratorSettings.OVERWORLD)
                .value().getRandomSource());
        OUT.printf("noise_settings minY=%d height=%d sizeHorizontal=%d sizeVertical=%d "
                        + "cellWidth=%d cellHeight=%d%n",
                ns.minY(), ns.height(), ns.noiseSizeHorizontal(), ns.noiseSizeVertical(),
                ns.getCellWidth(), ns.getCellHeight());
    }

    /** Holds the two registry captures plus the built overworld settings. */
    record Built(Capture<NormalNoise.NoiseParameters> noise,
                 Capture<DensityFunction> density,
                 Capture<NoiseGeneratorSettings> settings) {
        NoiseGeneratorSettings overworld() {
            return settings.map.get(NoiseGeneratorSettings.OVERWORLD).value();
        }

        MapGetter<NormalNoise.NoiseParameters> noiseGetter() {
            return new MapGetter<>(noise.map);
        }
    }

    static Built bootstrap() {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();

        Capture<NormalNoise.NoiseParameters> noise = new Capture<>();
        NoiseData.bootstrap(noise);
        Capture<DensityFunction> density = new Capture<>();
        density.lookups.put(Registries.NOISE, new MapGetter<>(noise.map));
        density.lookups.put(Registries.DENSITY_FUNCTION, new MapGetter<>(density.map));
        NoiseRouterData.bootstrap(density);
        Capture<NoiseGeneratorSettings> settingsCapture = new Capture<>();
        settingsCapture.lookups.put(Registries.NOISE, new MapGetter<>(noise.map));
        settingsCapture.lookups.put(Registries.DENSITY_FUNCTION, new MapGetter<>(density.map));
        NoiseGeneratorSettings.bootstrap(settingsCapture);
        return new Built(noise, density, settingsCapture);
    }

    static NoiseRouter buildOverworldRouter() {
        net.minecraft.SharedConstants.tryDetectVersion();
        net.minecraft.server.Bootstrap.bootStrap();

        // 1. Noise parameters. Nothing depends on anything yet.
        Capture<NormalNoise.NoiseParameters> noise = new Capture<>();
        NoiseData.bootstrap(noise);

        // 2. Density functions. They reference noise parameters by key.
        Capture<DensityFunction> density = new Capture<>();
        density.lookups.put(Registries.NOISE, new MapGetter<>(noise.map));
        density.lookups.put(Registries.DENSITY_FUNCTION, new MapGetter<>(density.map));
        NoiseRouterData.bootstrap(density);

        // 3. Generator settings, which build the router itself.
        Capture<NoiseGeneratorSettings> settings = new Capture<>();
        settings.lookups.put(Registries.NOISE, new MapGetter<>(noise.map));
        settings.lookups.put(Registries.DENSITY_FUNCTION, new MapGetter<>(density.map));
        NoiseGeneratorSettings.bootstrap(settings);

        Holder.Reference<NoiseGeneratorSettings> overworld =
                settings.map.get(NoiseGeneratorSettings.OVERWORLD);
        if (overworld == null) {
            throw new IllegalStateException("OVERWORLD settings were not registered");
        }
        return overworld.value().noiseRouter();
    }

    /**
     * Chunk-level parity: vanilla's *real* chunk pipeline, driven exactly the way
     * {@code NoiseBasedChunkGenerator.iterateNoiseColumn} drives it.
     *
     * <p>Two things are emitted:
     * <ul>
     *   <li>{@code router_seeded_density} — the raw final density at block
     *       coordinates, with {@code RandomState}'s own noise seeding. This isolates
     *       the seeding rule from the cell grid.</li>
     *   <li>{@code column} — the solid/air decision per block, taken from
     *       {@code NoiseChunk}, which is the only way to exercise the 4 x 8 x 4 cell
     *       grid and its trilinear interpolation.</li>
     * </ul>
     *
     * <p>A column of names is written to {@code -Dmcgen.names} so the Rust side can
     * seed its noises identically; without them it cannot reproduce this at all.
     */
    static void column() throws Exception {
        Built built = bootstrap();
        NoiseGeneratorSettings settings = built.overworld();
        NoiseSettings ns = settings.noiseSettings();

        long seed = 0L;

        // A copy of the overworld settings with aquifers switched off. With aquifers on,
        // the block verdict is not a pure function of the density sign: near the water
        // line the aquifer substitutes water for air, and the generator folds a null
        // (no fluid) into the default block. Disabled, the verdict collapses to exactly
        // `density > 0`, which is what the Rust side models.
        NoiseGeneratorSettings noAquifers = new NoiseGeneratorSettings(
                settings.noiseSettings(),
                settings.defaultBlock(),
                settings.defaultFluid(),
                settings.noiseRouter(),
                settings.surfaceRule(),
                settings.spawnTarget(),
                settings.seaLevel(),
                settings.disableMobGeneration(),
                false,
                settings.oreVeinsEnabled(),
                settings.useLegacyRandomSource());

        RandomState randomState = RandomState.create(noAquifers, built.noiseGetter(), seed);
        NoiseRouter router = randomState.router();

        List<Map.Entry<String, DensityFunction>> routes = List.of(
                Map.entry("router_temperature", router.temperature()),
                Map.entry("router_vegetation", router.vegetation()),
                Map.entry("router_continents", router.continents()),
                Map.entry("router_erosion", router.erosion()),
                Map.entry("router_depth", router.depth()),
                Map.entry("router_ridges", router.ridges()),
                Map.entry("router_barrier", router.barrierNoise()),
                Map.entry("router_fluid_floodedness", router.fluidLevelFloodednessNoise()),
                Map.entry("router_fluid_spread", router.fluidLevelSpreadNoise()),
                Map.entry("router_lava", router.lavaNoise()),
                Map.entry("router_vein_toggle", router.veinToggle()),
                Map.entry("router_vein_ridged", router.veinRidged()),
                Map.entry("router_vein_gap", router.veinGap()),
                Map.entry("router_initial_density", router.initialDensityWithoutJaggedness()),
                Map.entry("router_final_density", router.finalDensity()));

        // One line per noise *occurrence*, per route, in the order the graph is walked.
        //
        // Parameters alone cannot identify a noise: `final_density` collapses 25 distinct
        // registries into 12 parameter sets, one of which is shared by eight different
        // registries. The Rust parser consumes these left to right and checks each against
        // the parameters it parsed, so a traversal-order mismatch fails loudly.
        StringBuilder occurrences = new StringBuilder();
        java.util.TreeMap<String, NormalNoise.NoiseParameters> byName = new java.util.TreeMap<>();
        for (Map.Entry<String, DensityFunction> route : routes) {
            route.getValue().mapAll(new DensityFunction.Visitor() {
                @Override
                public DensityFunction apply(DensityFunction f) {
                    return f;
                }

                @Override
                public DensityFunction.NoiseHolder visitNoise(DensityFunction.NoiseHolder holder) {
                    NormalNoise.NoiseParameters params = holder.noiseData().value();
                    String name = holder.noiseData().unwrapKey()
                            .orElseThrow(() -> new IllegalStateException("unkeyed noise holder"))
                            .location().toString();
                    byName.putIfAbsent(name, params);
                    occurrences.append(route.getKey()).append(' ')
                            .append(params.firstOctave()).append(' ');
                    List<Double> amps = params.amplitudes();
                    for (int k = 0; k < amps.size(); k++) {
                        if (k > 0) {
                            occurrences.append(',');
                        }
                        occurrences.append(Long.toHexString(
                                Double.doubleToRawLongBits(amps.get(k))));
                    }
                    occurrences.append(' ').append(name).append('\n');
                    return holder;
                }
            });
        }

        // Clamp bounds, in `mapAll` post-order — the order the Rust parser finishes each
        // clamp. A `clamp` node's JSON carries only its input, and the bounds cannot be
        // recovered from the text, so they are handed over separately.
        StringBuilder clampTable = new StringBuilder();
        int[] seen = {0};
        java.util.Set<String> namesSeen = new java.util.TreeSet<>();
        // NOTE: this cannot use `mapAll`. That visitor never sees `Clamp`, `Mapped` or
        // `MulOrAdd` at all — they are `PureTransformer`s and their `mapAll` does not apply
        // the visitor to themselves. A reflective walk does reach them, and pre-order is the
        // order the Rust parser consumes bounds in.
        for (Map.Entry<String, DensityFunction> route : routes) {
            List<double[]> bounds = new ArrayList<>();
            collectClamps(route.getValue(), bounds);
            for (double[] b : bounds) {
                // Raw bit patterns, not `Double.toHexString` (which emits Java's 0x1.0p-3
                // float form, unreadable as an integer).
                clampTable.append(route.getKey()).append(' ')
                        .append(Long.toHexString(Double.doubleToRawLongBits(b[0]))).append(' ')
                        .append(Long.toHexString(Double.doubleToRawLongBits(b[1]))).append('\n');
            }
        }
        String clampsPath = System.getProperty("mcgen.clamps");
        if (clampsPath != null) {
            java.nio.file.Files.writeString(
                    java.nio.file.Path.of(clampsPath), clampTable.toString());
        }
        System.err.printf("note: clamp scan: %d apply calls, distinct classes: %s%n",
                seen[0], namesSeen);
        System.err.printf("note: clamp table has %d entries%n",
                clampTable.toString().lines().filter(l -> !l.isBlank()).count());

        String namesPath = System.getProperty("mcgen.names");
        if (namesPath != null) {
            java.nio.file.Files.writeString(
                    java.nio.file.Path.of(namesPath), occurrences.toString());
        }

        net.minecraft.world.level.levelgen.PositionalRandomFactory factory =
                net.minecraft.world.level.levelgen.WorldgenRandom.Algorithm.XOROSHIRO
                        .newInstance(seed).forkPositional();

        // (0) The derivation chain alone, and (0b) one value per noise. Both indexed by
        // the name's hash so neither side depends on iteration order.
        for (Map.Entry<String, NormalNoise.NoiseParameters> e : byName.entrySet()) {
            long draw = factory.fromHashOf(e.getKey()).nextLong();
            emit("noise_seed", seed, e.getKey().hashCode(), Double.longBitsToDouble(draw));
        }
        if ("noise_seed".equals(System.getProperty("mcgen.only"))) {
            return;
        }

        for (Map.Entry<String, NormalNoise.NoiseParameters> e : byName.entrySet()) {
            NormalNoise nn = NormalNoise.create(factory.fromHashOf(e.getKey()), e.getValue());
            emit("noise_value", seed, e.getKey().hashCode(), nn.getValue(0.0, 0.0, 0.0));
        }
        // The noises `RandomState` itself hands out, as opposed to ones rebuilt from a
        // hand-made factory. If these disagree, the settings copy or the random-source
        // algorithm is at fault rather than any downstream arithmetic.
        Map<String, NormalNoise> rsNoiseByName = new LinkedHashMap<>();
        Map<String, NormalNoise.NoiseParameters> paramsByName = new LinkedHashMap<>();
        router.finalDensity().mapAll(new DensityFunction.Visitor() {
            @Override
            public DensityFunction apply(DensityFunction f) {
                return f;
            }

            @Override
            public DensityFunction.NoiseHolder visitNoise(DensityFunction.NoiseHolder holder) {
                ResourceKey<NormalNoise.NoiseParameters> key = holder.noiseData().unwrapKey()
                        .orElseThrow();
                rsNoiseByName.putIfAbsent(key.location().toString(),
                        randomState.getOrCreateNoise(key));
                paramsByName.putIfAbsent(key.location().toString(), holder.noiseData().value());
                return holder;
            }
        });
        // The Rust side compares positionally and filters to the names `final_density` uses,
        // which is exactly the set `RandomState` wired here. Sort by the same key hash so the
        // unordered map cannot reorder the comparison.
        var rsNoiseSorted = new java.util.ArrayList<>(rsNoiseByName.entrySet());
        rsNoiseSorted.sort(java.util.Comparator.comparingInt(e -> e.getKey().hashCode()));
        for (Map.Entry<String, NormalNoise> e : rsNoiseSorted) {
            emit("noise_value_rs", seed, e.getKey().hashCode(), e.getValue().getValue(0.0, 0.0, 0.0));
        }

        if ("noise_value".equals(System.getProperty("mcgen.only"))) {
            return;
        }


        // The order in which `final_density` first meets each distinct noise. Compared
        // against the Rust parser's own interning order, this pins the association; a
        // parameter-only check cannot, because several distinct registries share
        // identical parameters.
        java.util.List<String> fdOccurrences = new ArrayList<>();
        router.finalDensity().mapAll(new DensityFunction.Visitor() {
            @Override
            public DensityFunction apply(DensityFunction f) {
                return f;
            }

            @Override
            public DensityFunction.NoiseHolder visitNoise(DensityFunction.NoiseHolder holder) {
                fdOccurrences.add(holder.noiseData().unwrapKey().orElseThrow().location().toString());
                return holder;
            }
        });
        int fdIndex = 0;
        for (String name : fdOccurrences) {
            emit("fd_occ_order", seed, fdIndex++, name.hashCode());
        }

        DensityFunction.FunctionContext[] contexts = new DensityFunction.FunctionContext[24];
        for (int i = 0; i < 24; i++) {
            int bx = blockX(i);
            int by = blockY(i);
            int bz = blockZ(i);
            contexts[i] = new DensityFunction.FunctionContext() {
                @Override public int blockX() { return bx; }
                @Override public int blockY() { return by; }
                @Override public int blockZ() { return bz; }
            };
        }

        // (0c) One route at a time, so a failure localises to a graph.
        {
            for (Map.Entry<String, DensityFunction> route : routes) {
                for (int i = 0; i < 24; i++) {
                    emit(route.getKey(), seed, i, route.getValue().compute(contexts[i]));
                }
            }
        }

        // (0f) Reconstruct `final_density` from its own JSON and feed it `RandomState`'s
        // noises in traversal order — precisely what the Rust side does. If Java cannot
        // reproduce its own direct evaluation this way, the JSON round trip is the
        // problem, not the Rust port.
        String routerJson = System.getenv("MCGEN_ROUTER_JSON");
        if (routerJson != null) {
            String fdJson = null;
            for (String line : java.nio.file.Files.readAllLines(
                    java.nio.file.Path.of(routerJson))) {
                if (line.startsWith("router_final_density\t")) {
                    fdJson = line.substring(line.indexOf('\t') + 1);
                    break;
                }
            }
            if (fdJson == null) {
                throw new IllegalStateException("no router_final_density line in " + routerJson);
            }
            DensityFunction reparsed = DensityFunction.DIRECT_CODEC
                    .parse(com.mojang.serialization.JsonOps.INSTANCE,
                            com.google.gson.JsonParser.parseString(fdJson))
                    .getOrThrow(msg -> new IllegalStateException(msg));
            int[] cursor = {0};
            DensityFunction rewired = reparsed.mapAll(new DensityFunction.Visitor() {
                @Override
                public DensityFunction apply(DensityFunction f) {
                    return f;
                }

                @Override
                public DensityFunction.NoiseHolder visitNoise(DensityFunction.NoiseHolder holder) {
                    if (cursor[0] >= fdOccurrences.size()) {
                        throw new IllegalStateException(
                                "the reparsed graph has more noise occurrences than the real one");
                    }
                    String name = fdOccurrences.get(cursor[0]++);
                    NormalNoise nn = rsNoiseByName.get(name);
                    if (nn == null) {
                        throw new IllegalStateException("no RandomState noise for " + name);
                    }
                    return new DensityFunction.NoiseHolder(holder.noiseData(), nn);
                }
            });
            System.err.printf("note: reparsed graph consumed %d of %d noise occurrences%n",
                    cursor[0], fdOccurrences.size());


            // Is the encoded form even a fixed point of the codec? If `encode(parse(j))`
            // differs from `j`, the lossy node is visible in the two texts.
            var j1 = DensityFunction.DIRECT_CODEC
                    .encodeStart(com.mojang.serialization.JsonOps.INSTANCE, router.finalDensity())
                    .getOrThrow(msg -> new IllegalStateException(msg));
            var j2 = DensityFunction.DIRECT_CODEC
                    .encodeStart(com.mojang.serialization.JsonOps.INSTANCE, reparsed)
                    .getOrThrow(msg -> new IllegalStateException(msg));
            java.nio.file.Files.writeString(java.nio.file.Path.of("/tmp/fd_j1.json"), j1.toString());
            java.nio.file.Files.writeString(java.nio.file.Path.of("/tmp/fd_j2.json"), j2.toString());
            System.err.printf("note: encode(direct)=%d chars, encode(parse(j))=%d chars, equal=%b%n",
                    j1.toString().length(), j2.toString().length(), j1.equals(j2));

            var reparseContexts =
                    new DensityFunction.FunctionContext[ns.height()];
            // The one place the walk could only compare by identity. `PerlinNoise` does not
            // override `equals`, so `Objects.equals` reports them as differing even when they
            // compute the same. Compare the BlendedNoise nodes by value instead.
            DensityFunction liveBlended = findBlended(router.finalDensity());
            DensityFunction repBlended = findBlended(rewired);
            if (liveBlended != null && repBlended != null) {
                var bctxs = probeContexts();
                double[] lb = probeValues(liveBlended, bctxs);
                double[] rb = probeValues(repBlended, bctxs);
                int bn = 0;
                for (int i = 0; i < lb.length; i++) {
                    if (Double.doubleToRawLongBits(lb[i]) != Double.doubleToRawLongBits(rb[i])) {
                        bn++;
                    }
                }
                System.err.printf(
                        "note: BLENDED live=%s reparse=%s  values differ %d/%d  (first live=%s reparse=%s)%n",
                        liveBlended.getClass().getSimpleName(),
                        repBlended.getClass().getSimpleName(), bn, lb.length,
                        Double.toHexString(lb[0]), Double.toHexString(rb[0]));
            } else {
                System.err.printf("note: BLENDED not found live=%b reparse=%b%n",
                        liveBlended != null, repBlended != null);
            }

            // Diagnostics must never take the oracle down: an exception here has no message
            // (Minecraft's stderr swallows it) and would silently lose every later section.
            try {
                diffGraphs(router.finalDensity(), rewired, "rewired", 0, new int[] {8});
            } catch (Throwable t) {
                System.err.printf("note: graph diff aborted: %s%n", t);
            }

            int[] reparseResidues = {0, 1, 2, 3, 15};
            // The live graph's values on the same columns, as Java-seen ground truth.
            double[][] liveValues = new double[reparseResidues.length * reparseResidues.length][];
            int fdColumn = 0;
            for (int lx : reparseResidues) {
                for (int lz : reparseResidues) {
                    liveValues[fdColumn] = new double[ns.height()];
                    for (int ly = 0; ly < ns.height(); ly++) {
                        int bx = lx;
                        int by = ns.minY() + ly;
                        int bz = lz;
                        liveValues[fdColumn][ly] = router.finalDensity().compute(
                                new DensityFunction.FunctionContext() {
                                    @Override public int blockX() { return bx; }
                                    @Override public int blockY() { return by; }
                                    @Override public int blockZ() { return bz; }
                                });
                    }
                    fdColumn++;
                }
            }
            int reparseMismatches = 0;
            fdColumn = 0;
            for (int lx : reparseResidues) {
                for (int lz : reparseResidues) {
                    for (int ly = 0; ly < ns.height(); ly++) {
                        int bx = lx;
                        int by = ns.minY() + ly;
                        int bz = lz;
                        reparseContexts[ly] = new DensityFunction.FunctionContext() {
                            @Override public int blockX() { return bx; }
                            @Override public int blockY() { return by; }
                            @Override public int blockZ() { return bz; }
                        };
                    }
                    for (int ly = 0; ly < ns.height(); ly++) {
                        double v = rewired.compute(reparseContexts[ly]);
                        if (Double.doubleToRawLongBits(v)
                                != Double.doubleToRawLongBits(liveValues[fdColumn][ly])) {
                            reparseMismatches++;
                        }
                    }
                    fdColumn++;
                }
            }
            System.err.printf(
                    "note: JAVA-vs-JAVA: live final_density vs its own reparse differ at %d of %d points%n",
                    reparseMismatches, reparseResidues.length * reparseResidues.length * ns.height());
        }

        // (0g) The candidate fix. `DIRECT_CODEC` re-wraps nodes in `HolderHolder` on parse
        // even though the text is byte-identical, and those wrappers do not compute the
        // same as what they replace. Unwrapping every `HolderHolder` before encoding gives
        // the codec nothing to re-wrap. Three value sets, so the claim is checked rather
        // than assumed: live vs unwrapped vs re-parsed-of-unwrapped.
        {
            var holderField = Class
                    .forName("net.minecraft.world.level.levelgen.DensityFunctions$HolderHolder")
                    .getDeclaredField("function");
            holderField.setAccessible(true);
            DensityFunction unwrapped = router.finalDensity().mapAll(new DensityFunction.Visitor() {
                @Override
                public DensityFunction apply(DensityFunction f) {
                    if (!f.getClass().getSimpleName().equals("HolderHolder")) {
                        return f;
                    }
                    try {
                        Object holder = holderField.get(f);
                        return (DensityFunction) holder.getClass().getMethod("value").invoke(holder);
                    } catch (ReflectiveOperationException e) {
                        throw new IllegalStateException(e);
                    }
                }

                @Override
                public DensityFunction.NoiseHolder visitNoise(DensityFunction.NoiseHolder holder) {
                    return holder;
                }
            });
            var unwrappedTree = DensityFunction.DIRECT_CODEC
                    .encodeStart(com.mojang.serialization.JsonOps.INSTANCE, unwrapped)
                    .getOrThrow(msg -> new IllegalStateException(msg));
            DensityFunction unwrappedReparsed = DensityFunction.DIRECT_CODEC
                    .parse(com.mojang.serialization.JsonOps.INSTANCE, unwrappedTree)
                    .getOrThrow(msg -> new IllegalStateException(msg));

            var ctxs = probeContexts();
            double[] live = probeValues(router.finalDensity(), ctxs);
            double[] unw = probeValues(unwrapped, ctxs);
            double[] rep = probeValues(unwrappedReparsed, ctxs);
            int dUnw = 0;
            int dRep = 0;
            for (int i = 0; i < ctxs.length; i++) {
                if (Double.doubleToRawLongBits(live[i]) != Double.doubleToRawLongBits(unw[i])) {
                    dUnw++;
                }
                if (Double.doubleToRawLongBits(live[i]) != Double.doubleToRawLongBits(rep[i])) {
                    dRep++;
                }
            }
            System.err.printf(
                    "note: live vs unwrapped = %d/%d differ; live vs parse(encode(unwrapped)) = %d/%d%n",
                    dUnw, ctxs.length, dRep, ctxs.length);

            // Does the *text* change when the holder wrappers are removed? If it does, the
            // current corpus describes the wrappers rather than the graph, and a parser
            // that does not re-wrap (the Rust one) would be reading the wrong thing.
            var liveTree = DensityFunction.DIRECT_CODEC
                    .encodeStart(com.mojang.serialization.JsonOps.INSTANCE, router.finalDensity())
                    .getOrThrow(msg -> new IllegalStateException(msg));
            System.err.printf("note: encode(live)=%d chars, encode(unwrapped)=%d chars, equal=%b%n",
                    liveTree.toString().length(), unwrappedTree.toString().length(),
                    liveTree.equals(unwrappedTree));
            java.nio.file.Files.writeString(
                    java.nio.file.Path.of("/tmp/fd_unwrapped.json"), unwrappedTree.toString());
        }

        // (1) The cell grid, through the real NoiseChunk.
        int cellWidth = ns.getCellWidth();
        int cellHeight = ns.getCellHeight();
        int minCellY = Mth.floorDiv(ns.minY(), cellHeight);
        int cellCountY = Mth.floorDiv(ns.height(), cellHeight);
        int cellCountXZ = 16 / cellWidth;

        // A picker that always reports *water*. This is not cosmetic: with an always-air
        // picker the aquifer bails out near sea level and returns null, which the caller
        // reads as "use the default block", i.e. solid — turning the solid/air verdict
        // into something other than the sign of the density and producing a ring of
        // spurious mismatches at the water line.
        Aquifer.FluidPicker picker = (fx, fy, fz) ->
                new Aquifer.FluidStatus(Integer.MAX_VALUE, Blocks.WATER.defaultBlockState());

        java.lang.reflect.Method getInterpolatedState =
                NoiseChunk.class.getDeclaredMethod("getInterpolatedState");
        getInterpolatedState.setAccessible(true);

        Class<?> markerClass = Class.forName(
                "net.minecraft.world.level.levelgen.DensityFunctions$BeardifierMarker");
        java.lang.reflect.Field instanceField = markerClass.getDeclaredField("INSTANCE");
        instanceField.setAccessible(true);
        DensityFunctions.BeardifierOrMarker beardifier =
                (DensityFunctions.BeardifierOrMarker) instanceField.get(null);

        // (2) The raw, uninterpolated density on the same columns. Comparing this against
        // the cell-grid result separates "the density is wrong" from "the interpolation
        // is wrong" — a distinction the block verdict alone cannot make.

        DensityFunction.FunctionContext[] columnContexts =
                new DensityFunction.FunctionContext[ns.height()];

        // The fair baseline: one `NoiseChunk` for the whole chunk, driven exactly as
        // `NoiseBasedChunkGenerator.fillFromNoise` drives it, with the block-state rule reached
        // through a `MethodHandle` rather than `Method.invoke`. Both of those matter. The earlier
        // attempt built a chunk per *column* and reflectively invoked per block, and duly
        // reported 70 us per block, which measured the harness rather than the density.
        try {
            java.lang.invoke.MethodHandle state =
                    java.lang.invoke.MethodHandles.lookup().unreflect(getInterpolatedState);
            double sink = 0.0;
            long nanos = 0L;
            int blocks = 0;
            for (int pass = 0; pass < 3; pass++) {
                // A fresh chunk per pass: `initializeForFirstCellX` may only be called once,
                // and vanilla builds one `NoiseChunk` per chunk anyway.
                NoiseChunk c = new NoiseChunk(cellCountXZ, randomState, 0, 0, ns,
                        beardifier, noAquifers, picker, Blender.empty());
                long t0 = System.nanoTime();
                c.initializeForFirstCellX();
                for (int cellX = 0; cellX < cellCountXZ; cellX++) {
                    c.advanceCellX(cellX);
                    for (int cellZ = 0; cellZ < cellCountXZ; cellZ++) {
                        for (int cellY = cellCountY - 1; cellY >= 0; cellY--) {
                            c.selectCellYZ(cellY, cellZ);
                            for (int yInCell = cellHeight - 1; yInCell >= 0; yInCell--) {
                                int blockY = (minCellY + cellY) * cellHeight + yInCell;
                                c.updateForY(blockY, yInCell / (double) cellHeight);
                                for (int xInCell = 0; xInCell < cellWidth; xInCell++) {
                                    c.updateForX(cellX * cellWidth + xInCell,
                                            xInCell / (double) cellWidth);
                                    for (int zInCell = 0; zInCell < cellWidth; zInCell++) {
                                        c.updateForZ(cellZ * cellWidth + zInCell,
                                                zInCell / (double) cellWidth);
                                        try {
                                            sink += ((BlockState) state.invoke(c)) == null
                                                    ? 1.0 : 0.0;
                                        } catch (Throwable t) {
                                            throw new IllegalStateException(t);
                                        }
                                        blocks++;
                                    }
                                }
                            }
                        }
                    }
                }
                nanos = System.nanoTime() - t0;
            }
            System.err.printf(
                    "note: vanilla chunk path: %d block states in %d ms (%.1f ns/block) sink=%s%n",
                    blocks, nanos / 1_000_000, (double) nanos / blocks,
                    Double.toHexString(sink));
        } catch (Throwable t) {
            System.err.printf("note: vanilla chunk path measurement failed: %s%n", t);
        }

        // Every column of the chunk, not a sample of five. The old five-column set happened
        // to be satisfied by a cell grid that coarse-sampled the *whole* density, so it could
        // not tell the two algorithms apart; 256 columns can.
        int[] residues = new int[16];
        for (int i = 0; i < residues.length; i++) {
            residues[i] = i;
        }
        int columnIndex = 0;
        for (int lx : residues) {
            for (int lz : residues) {
                for (int ly = 0; ly < ns.height(); ly++) {
                    int bx = lx;
                    int by = ns.minY() + ly;
                    int bz = lz;
                    columnContexts[ly] = new DensityFunction.FunctionContext() {
                        @Override public int blockX() { return bx; }
                        @Override public int blockY() { return by; }
                        @Override public int blockZ() { return bz; }
                    };
                }
                // Every route at the same coordinates, so the first one to diverge
                // localises the fault without any guesswork.
                for (int ly = 0; ly < ns.height(); ly++) {
                    int index = columnIndex * ns.height() + ly;
                    for (Map.Entry<String, DensityFunction> route : routes) {
                        emit("rawcol_" + route.getKey(), seed, index,
                                route.getValue().compute(columnContexts[ly]));
                    }
                }
                columnIndex++;
            }
        }
        columnIndex = 0;
        for (int lx : residues) {
            for (int lz : residues) {
                int cellX = Math.floorDiv(lx, cellWidth);
                int cellZ = Math.floorDiv(lz, cellWidth);
                double fracX = Math.floorMod(lx, cellWidth) / (double) cellWidth;
                double fracZ = Math.floorMod(lz, cellWidth) / (double) cellWidth;

                NoiseChunk chunk = new NoiseChunk(cellCountXZ, randomState,
                        cellX * cellWidth, cellZ * cellWidth, ns,
                        beardifier, noAquifers, picker, Blender.empty());
                chunk.initializeForFirstCellX();
                chunk.advanceCellX(0);

                // The raw interpolated density, not just its sign. `NoiseInterpolator.compute`
                // returns the interpolated value when handed the chunk itself as the context
                // (any other context is delegated straight through to the wrapped function).
                DensityFunction interpolator = null;
                try {
                    var listField = NoiseChunk.class.getDeclaredField("interpolators");
                    listField.setAccessible(true);
                    var fillerField = Class
                            .forName("net.minecraft.world.level.levelgen.NoiseChunk$NoiseInterpolator")
                            .getDeclaredField("noiseFiller");
                    fillerField.setAccessible(true);
                    // `NoiseChunk` does not interpolate `finalDensity` itself: it first wraps it
                    // in `DensityFunctions.blendDensity`, and that `BlendDensity` is what gets an
                    // interpolator. With `Blender.empty()` the wrapper is a passthrough, so its
                    // interpolated value is the one the Rust cell grid should reproduce.
                    // Match by reachability, not by guesswork: there are several `BlendDensity`
                    // interpolators and the obvious "the one whose filler is finalDensity" test
                    // finds none, because `NoiseChunk` wraps it first.
                    for (Object o : (java.util.List<?>) listField.get(chunk)) {
                        DensityFunction filler = (DensityFunction) fillerField.get(o);
                        if (reaches(filler, router.finalDensity(), new int[] {400})) {
                            interpolator = (DensityFunction) o;
                        }
                    }
                } catch (ReflectiveOperationException e) {
                    System.err.printf("note: could not reach the interpolators: %s%n", e);
                }
                if (cellX == 0 && cellZ == 0) {
                    try {
                        var lf = NoiseChunk.class.getDeclaredField("interpolators");
                        lf.setAccessible(true);
                        java.util.List<?> list = (java.util.List<?>) lf.get(chunk);
                        var ff = Class
                                .forName("net.minecraft.world.level.levelgen.NoiseChunk$NoiseInterpolator")
                                .getDeclaredField("noiseFiller");
                        ff.setAccessible(true);
                        StringBuilder sb = new StringBuilder();
                        for (Object o : list) {
                            sb.append(fillerLabel(((DensityFunction) ff.get(o)))).append(' ');
                        }
                        System.err.printf("note: %d interpolators: %s%n", list.size(), sb);
                    } catch (ReflectiveOperationException e) {
                        System.err.printf("note: interpolator count failed: %s%n", e);
                    }
                }
                if (false) {
                    try {
                        var lf = NoiseChunk.class.getDeclaredField("interpolators");
                        lf.setAccessible(true);
                        var ff = Class
                                .forName("net.minecraft.world.level.levelgen.NoiseChunk$NoiseInterpolator")
                                .getDeclaredField("noiseFiller");
                        ff.setAccessible(true);
                        for (Object o : (java.util.List<?>) lf.get(chunk)) {
                            Object f2 = ff.get(o);
                            if (reaches((DensityFunction) f2, router.finalDensity(), new int[] {400})) {
                                System.err.printf("note: chosen interpolator wraps %s%n",
                                        f2.getClass().getSimpleName());
                            }
                        }
                    } catch (ReflectiveOperationException e) {
                        System.err.printf("note: interpolator listing failed: %s%n", e);
                    }
                }
                if (false) {
                    // What does the interpolator actually wrap? At a cell corner the interpolated
                    // value is the sampled value, and it disagrees with the live `finalDensity`,
                    // so something is being applied in between.
                    try {
                        var listField = NoiseChunk.class.getDeclaredField("interpolators");
                        listField.setAccessible(true);
                        var fillerField = Class
                                .forName("net.minecraft.world.level.levelgen.NoiseChunk$NoiseInterpolator")
                                .getDeclaredField("noiseFiller");
                        fillerField.setAccessible(true);
                        for (Object o : (java.util.List<?>) listField.get(chunk)) {
                            Object filler = fillerField.get(o);
                            if (filler.getClass().getSimpleName().equals("BlendDensity")) {
                                Object inner = componentOf(filler, "input");
                                System.err.printf("note: blendDensity wraps %s%n",
                                        inner == null ? "?" : inner.getClass().getSimpleName());
                                Object deeper = inner == null ? null : componentOf(inner, "input");
                                System.err.printf("note:   and that wraps %s%n",
                                        deeper == null ? "?" : deeper.getClass().getSimpleName());
                            }
                        }
                    } catch (ReflectiveOperationException e) {
                        System.err.printf("note: blendDensity introspection failed: %s%n", e);
                    }
                }
                if (interpolator == null && cellX == 0 && cellZ == 0) {
                    System.err.printf("note: no interpolator for final_density; emitting signs only%n");
                    try {
                        var listField = NoiseChunk.class.getDeclaredField("interpolators");
                        listField.setAccessible(true);
                        var fillerField = Class
                                .forName("net.minecraft.world.level.levelgen.NoiseChunk$NoiseInterpolator")
                                .getDeclaredField("noiseFiller");
                        fillerField.setAccessible(true);
                        for (Object o : (java.util.List<?>) listField.get(chunk)) {
                            Object filler = fillerField.get(o);
                            String txt = filler.toString();
                            System.err.printf("      interpolator wraps %s : %s%n",
                                    filler.getClass().getSimpleName(),
                                    txt.substring(0, Math.min(90, txt.length())));
                        }
                    } catch (ReflectiveOperationException e) {
                        System.err.printf("      listing failed: %s%n", e);
                    }
                }
                // The cell machine must be walked top-down; the output is emitted
                // bottom-up afterwards so the two sides' files line up.
                double[] bits = new double[ns.height()];
                for (int cellY = cellCountY - 1; cellY >= 0; cellY--) {
                    chunk.selectCellYZ(cellY, 0);
                    for (int yInCell = cellHeight - 1; yInCell >= 0; yInCell--) {
                        int blockY = (minCellY + cellY) * cellHeight + yInCell;
                        // `NoiseInterpolator.compute` takes its fractions from `inCellX/width`,
                        // `inCellY/height`, `inCellZ/width` — the *in-cell* offsets, not the
                        // absolute block position. Passing the block position makes the lerp
                        // weight run up to 3.75 instead of 0.75.
                        chunk.updateForY(yInCell, yInCell / (double) cellHeight);
                        chunk.updateForX(Math.floorMod(lx, cellWidth), fracX);
                        chunk.updateForZ(Math.floorMod(lz, cellWidth), fracZ);

                        // null means the aquifer declined to place a fluid, which the
                        // generator resolves to the default block. So null is exactly
                        // "density > 0", making this a pure test of the density sign.
                        // Sign, not raw value: `NoiseChunk` gives `finalDensity` no interpolator
                        // at all, so there is no single vanilla value to compare the cell grid
                        // against. See the README.
                        bits[blockY - ns.minY()] =
                                ((BlockState) getInterpolatedState.invoke(chunk)) == null ? 1.0 : 0.0;
                    }
                }
                for (int localY = 0; localY < ns.height(); localY++) {
                    boolean solid = bits[localY] != 0.0;
                    emit("column", seed,
                            ((columnIndex * ns.height()) + localY) * 2 + (solid ? 1 : 0),
                            bits[localY]);
                }
                columnIndex++;
            }
        }
    }

    /** Probe coordinates for the round-trip bisect: the chunk-local region that fails. */
    static net.minecraft.world.level.levelgen.DensityFunction.FunctionContext[] probeContexts() {
        var out = new net.minecraft.world.level.levelgen.DensityFunction.FunctionContext[250];
        int[] ys = {-64, 0, 39, 45, 54, 60, 63, 80, 150, 250};
        int k = 0;
        for (int lx : new int[] {0, 1, 2, 3, 15}) {
            for (int lz : new int[] {0, 1, 2, 3, 15}) {
                for (int y : ys) {
                    int bx = lx;
                    int by = y;
                    int bz = lz;
                    out[k++] = new net.minecraft.world.level.levelgen.DensityFunction.FunctionContext() {
                        @Override public int blockX() { return bx; }
                        @Override public int blockY() { return by; }
                        @Override public int blockZ() { return bz; }
                    };
                }
            }
        }
        return out;
    }

    static double[] probeValues(net.minecraft.world.level.levelgen.DensityFunction fn,
                                 net.minecraft.world.level.levelgen.DensityFunction.FunctionContext[] ctxs) {
        double[] out = new double[ctxs.length];
        for (int i = 0; i < ctxs.length; i++) {
            out[i] = fn.compute(ctxs[i]);
        }
        return out;
    }

    /**
     * Every density function reachable inside this object through record components and
     * lists. Generic on purpose: `Spline` hides its coordinates inside a `CubicSpline`,
     * so recursing through arbitrary containers is the only way to find them all.
     */
    static void collectFunctions(Object o, List<net.minecraft.world.level.levelgen.DensityFunction> out) {
        if (o == null) {
            return;
        }
        if (o instanceof net.minecraft.world.level.levelgen.DensityFunction d) {
            out.add(d);
            return;
        }
        if (o instanceof List<?> list) {
            for (Object e : list) {
                collectFunctions(e, out);
            }
            return;
        }
        Class<?> c = o.getClass();
        if (!c.isRecord()) {
            return;
        }
        for (java.lang.reflect.RecordComponent rc : c.getRecordComponents()) {
            try {
                var acc = rc.getAccessor();
                acc.setAccessible(true);
                collectFunctions(acc.invoke(o), out);
            } catch (ReflectiveOperationException e) {
                // Unreadable component: contributes nothing.
            }
        }
    }

    /** Child density functions of a node, found reflectively through its record components. */
    static List<net.minecraft.world.level.levelgen.DensityFunction> childrenOf(
            net.minecraft.world.level.levelgen.DensityFunction fn) {
        List<net.minecraft.world.level.levelgen.DensityFunction> out = new ArrayList<>();
        Class<?> c = fn.getClass();
        if (!c.isRecord()) {
            return out;
        }
        for (java.lang.reflect.RecordComponent rc : c.getRecordComponents()) {
            try {
                var acc = rc.getAccessor();
                acc.setAccessible(true);
                Object v = acc.invoke(fn);
                if (v instanceof net.minecraft.world.level.levelgen.DensityFunction) {
                    out.add((net.minecraft.world.level.levelgen.DensityFunction) v);
                } else {
                    collectFunctions(v, out);
                }
            } catch (ReflectiveOperationException e) {
                // Unreadable component: contributes no children.
            }
        }
        return out;
    }

    /** Clamp bounds, pre-order, by a reflective walk — `mapAll` cannot see these nodes. */
    static void collectClamps(net.minecraft.world.level.levelgen.DensityFunction fn,
                              List<double[]> out) {
        if (fn.getClass().getSimpleName().equals("Clamp")) {
            out.add(new double[] {fn.minValue(), fn.maxValue()});
        }
        for (var kid : childrenOf(fn)) {
            collectClamps(kid, out);
        }
    }

    /** A short, stable label for an interpolator's wrapped function. */
    static String fillerLabel(DensityFunction fn) {
        String n = fn.getClass().getSimpleName();
        return switch (n) {
            case "Noise" -> "Noise";
            case "ShiftedNoise" -> "ShiftedNoise";
            case "BlendedNoise" -> "BlendedNoise";
            case "Ap2" -> "Ap2";
            case "MulOrAdd" -> "MulOrAdd";
            case "Mapped" -> "Mapped";
            case "Spline" -> "Spline";
            case "WeirdScaledSampler" -> "WeirdScaledSampler";
            default -> n;
        };
    }

    /** Whether `target` appears anywhere inside `fn`, walking the reflective decomposition. */
    static boolean reaches(DensityFunction fn, DensityFunction target, int[] budget) {
        if (fn == target) {
            return true;
        }
        if (budget[0]-- <= 0) {
            return false;
        }
        for (Object c : componentOfAll(fn)) {
            if (c instanceof DensityFunction d && reaches(d, target, budget)) {
                return true;
            }
        }
        return false;
    }

    /** Every object reachable from a node through its fields, one level deep. */
    static List<Object> componentOfAll(Object o) {
        List<Object> out = new ArrayList<>();
        for (var e : fieldsOf(o)) {
            out.add(e.getValue());
        }
        return out;
    }

    /** The first `BlendedNoise` reachable from a node, or null. */
    static DensityFunction findBlended(DensityFunction fn) {
        if (fn.getClass().getSimpleName().equals("BlendedNoise")) {
            return fn;
        }
        for (var kid : childrenOf(fn)) {
            DensityFunction found = findBlended(kid);
            if (found != null) {
                return found;
            }
        }
        return null;
    }

    /** All instance fields of a node, as name/value pairs, records or plain classes alike. */
    static List<Map.Entry<String, Object>> fieldsOf(Object o) {
        List<Map.Entry<String, Object>> out = new ArrayList<>();
        var rcs = o.getClass().getRecordComponents();
        if (rcs != null) {
            for (var rc : rcs) {
                try {
                    var acc = rc.getAccessor();
                    acc.setAccessible(true);
                    out.add(Map.entry(rc.getName(), acc.invoke(o)));
                } catch (ReflectiveOperationException e) {
                    // unreadable component contributes nothing
                }
            }
            return out;
        }
        for (Class<?> c = o.getClass(); c != null; c = c.getSuperclass()) {
            for (var f : c.getDeclaredFields()) {
                if (java.lang.reflect.Modifier.isStatic(f.getModifiers())) {
                    continue;
                }
                try {
                    f.setAccessible(true);
                    out.add(Map.entry(f.getName(), f.get(o)));
                } catch (ReflectiveOperationException | RuntimeException e) {
                    // unreadable field contributes nothing
                }
            }
        }
        return out;
    }

    /** One named record component of a node, or null. */
    static Object componentOf(Object o, String name) {
        if (o == null) {
            return null;
        }
        // Records expose components; `MulOrAdd` is a plain final class and has none, so fall
        // back to the declared field of the same name.
        var rcs = o.getClass().getRecordComponents();
        if (rcs != null) {
            for (var rc : rcs) {
                if (rc.getName().equals(name)) {
                    try {
                        var acc = rc.getAccessor();
                        acc.setAccessible(true);
                        return acc.invoke(o);
                    } catch (ReflectiveOperationException e) {
                        System.err.printf("      componentOf(%s) failed: %s%n", name, e);
                        return null;
                    }
                }
            }
        }
        for (Class<?> c = o.getClass(); c != null; c = c.getSuperclass()) {
            try {
                var f = c.getDeclaredField(name);
                f.setAccessible(true);
                return f.get(o);
            } catch (NoSuchFieldException e) {
                // try the superclass
            } catch (IllegalAccessException e) {
                return null;
            }
        }
        return null;
    }

    /** Unwrap a `HolderHolder`, if that is what this is. */
    static net.minecraft.world.level.levelgen.DensityFunction unwrapHolder(
            net.minecraft.world.level.levelgen.DensityFunction fn) {
        if (fn == null || !fn.getClass().getSimpleName().equals("HolderHolder")) {
            return fn;
        }
        try {
            var hf = Class
                    .forName("net.minecraft.world.level.levelgen.DensityFunctions$HolderHolder")
                    .getDeclaredField("function");
            hf.setAccessible(true);
            Object h = hf.get(fn);
            return (net.minecraft.world.level.levelgen.DensityFunction)
                    h.getClass().getMethod("value").invoke(h);
        } catch (ReflectiveOperationException e) {
            return fn;
        }
    }

    /**
     * Walk two graphs together and report the first component that actually differs.
     *
     * <p>This is the tool the earlier bisect should have been. That one compared a subtree
     * against `parse(encode(subtree))`, which is a *different* graph from the subtree in
     * context, so it could report "no loss" while the two graphs disagreed. Comparing the
     * two graphs directly has no such blind spot.
     */
    static void diffGraphs(net.minecraft.world.level.levelgen.DensityFunction a,
                           net.minecraft.world.level.levelgen.DensityFunction b,
                           String path, int depth, int[] budget) {
        if (budget[0] <= 0 || depth > 40) {
            return;
        }
        a = unwrapHolder(a);
        b = unwrapHolder(b);
        if (a.getClass() != b.getClass()) {
            budget[0]--;
            System.err.printf("DIFF %s: class %s vs %s%n", path,
                    a.getClass().getSimpleName(), b.getClass().getSimpleName());
            // Both `MulOrAdd` and `Ap2` expose `argument1`/`argument2`, so a class mismatch
            // between them can still be compared component by component. Returning early
            // here is exactly how the previous pass missed the difference.
            // Both `MulOrAdd` and `Ap2` expose `argument1`/`argument2`, so the pair can be
            // compared and descended through. Record components with `setAccessible` are the
            // access path that works: these classes are package-private, so a plain
            // `getMethod(...).invoke(...)` throws IllegalAccessException.
            // `MulOrAdd` stores its operands as `argument` (a double) and `input`; `Ap2`
            // stores them as `argument1`/`argument2`. Line the two up explicitly.
            net.minecraft.world.level.levelgen.DensityFunction liveInput =
                    (net.minecraft.world.level.levelgen.DensityFunction)
                            unwrapHolder((net.minecraft.world.level.levelgen.DensityFunction)
                                    componentOf(a, "input"));
            net.minecraft.world.level.levelgen.DensityFunction liveArg =
                    (net.minecraft.world.level.levelgen.DensityFunction)
                            unwrapHolder((net.minecraft.world.level.levelgen.DensityFunction)
                                    componentOf(a, "argument1"));
            net.minecraft.world.level.levelgen.DensityFunction repInput =
                    (net.minecraft.world.level.levelgen.DensityFunction)
                            unwrapHolder((net.minecraft.world.level.levelgen.DensityFunction)
                                    componentOf(b, "argument2"));
            net.minecraft.world.level.levelgen.DensityFunction repArg =
                    (net.minecraft.world.level.levelgen.DensityFunction)
                            unwrapHolder((net.minecraft.world.level.levelgen.DensityFunction)
                                    componentOf(b, "argument1"));
            System.err.printf("      live  : argument=%s input=%s%n",
                    componentOf(a, "argument"),
                    liveInput == null ? "null" : liveInput.getClass().getSimpleName());
            System.err.printf("      reparse: argument1=%s argument2=%s%n",
                    repArg == null ? "null" : repArg.getClass().getSimpleName(),
                    repInput == null ? "null" : repInput.getClass().getSimpleName());
            if (liveInput != null && repInput != null) {
                diffGraphs(liveInput, repInput, path + "/input", depth + 1, budget);
            }
            if (liveArg != null && repArg != null) {
                diffGraphs(liveArg, repArg, path + "/argument1", depth + 1, budget);
            }
            return;
        }
        var ctxs = probeContexts();
        double[] va = probeValues(a, ctxs);
        double[] vb = probeValues(b, ctxs);
        int nd = 0;
        for (int i = 0; i < va.length; i++) {
            if (Double.doubleToRawLongBits(va[i]) != Double.doubleToRawLongBits(vb[i])) {
                nd++;
            }
        }
        if (nd == 0) {
            return;
        }

        // Records expose components; `Marker`, `MulOrAdd` and friends are plain classes and
        // return null here, which used to throw and silently end the walk at the first such
        // node. Read both shapes.
        boolean descended = false;
        for (var rc : fieldsOf(a)) {
            try {
                String name = rc.getKey();
                Object x = rc.getValue();
                Object y = componentOf(b, name);
                if (x instanceof net.minecraft.world.level.levelgen.DensityFunction da
                        && y instanceof net.minecraft.world.level.levelgen.DensityFunction db) {
                    descended = true;
                    diffGraphs(da, db, path + "/" + name, depth + 1, budget);
                } else if (!java.util.Objects.equals(x, y)) {
                    budget[0]--;
                    System.err.printf("DIFF %s/%s: scalar %s vs %s   (node %s, %d/%d values differ)%n",
                            path, name, x, y, a.getClass().getSimpleName(), nd, va.length);
                }
            } catch (RuntimeException e) {
                System.err.printf("DIFF %s: component read failed: %s%n", path, e);
            }
        }
        if (!descended && budget[0] > 0) {
            budget[0]--;
            System.err.printf("DIFF %s: %s has no density-function component, %d/%d values differ%n",
                    path, a.getClass().getSimpleName(), nd, va.length);
        }
    }

    /** Results go here, not to stdout: Minecraft's logger rewrites stdout. */
    static java.io.PrintStream OUT = System.out;

    static void emit(String kind, long seed, int index, double value) {
        OUT.printf("%s %d %d %016x%n", kind, seed, index,
                Double.doubleToRawLongBits(value));
    }

    static int blockX(int i) {
        return (i - 12) * 7 + 3;
    }

    static int blockY(int i) {
        return i * 5 - 40;
    }

    static int blockZ(int i) {
        return (12 - i) * 11 - 3;
    }

    public static void main(String[] args) throws Exception {
        String mode = args.length > 0 ? args[0] : "encode";
        if (mode.equals("settings")) {
            String p = System.getProperty("mcgen.out");
            if (p != null) {
                OUT = new java.io.PrintStream(new java.io.FileOutputStream(p), false, "UTF-8");
            }
            printNoiseSettings();
            return;
        }
        String outPath = System.getProperty("mcgen.out");
        if (outPath != null) {
            OUT = new java.io.PrintStream(
                    new java.io.FileOutputStream(outPath), false, "UTF-8");
        }
        if (mode.equals("column")) {
            column();
            return;
        }

        NoiseRouter router = buildOverworldRouter();

        List<Map.Entry<String, DensityFunction>> named = new ArrayList<>();
        named.add(Map.entry("router_temperature", router.temperature()));
        named.add(Map.entry("router_vegetation", router.vegetation()));
        named.add(Map.entry("router_continents", router.continents()));
        named.add(Map.entry("router_erosion", router.erosion()));
        named.add(Map.entry("router_depth", router.depth()));
        named.add(Map.entry("router_ridges", router.ridges()));
        named.add(Map.entry("router_barrier", router.barrierNoise()));
        named.add(Map.entry("router_fluid_floodedness", router.fluidLevelFloodednessNoise()));
        named.add(Map.entry("router_fluid_spread", router.fluidLevelSpreadNoise()));
        named.add(Map.entry("router_lava", router.lavaNoise()));
        named.add(Map.entry("router_vein_toggle", router.veinToggle()));
        named.add(Map.entry("router_vein_ridged", router.veinRidged()));
        named.add(Map.entry("router_vein_gap", router.veinGap()));
        named.add(Map.entry("router_initial_density", router.initialDensityWithoutJaggedness()));
        named.add(Map.entry("router_final_density", router.finalDensity()));

        // Initialise every noise holder with a real NormalNoise, exactly as
        // RandomState does. Without this the holders are inert and every
        // noise-driven node evaluates to zero.
        //
        // The seed rule is deliberately shared with the Rust side:
        //   seed = RandomSupport.seedFromHashOf(noiseKey).seedLo()
        // which Rust already reproduces bit-for-bit as `seed_from_hash_of`.
        DensityFunction.Visitor init = new DensityFunction.Visitor() {
            @Override
            public DensityFunction apply(DensityFunction f) {
                // Registry references arrive as `HolderHolder`, whose codec throws
                // ("Calling .codec() on HolderHolder") so the graph cannot be
                // serialised while any remain. After `mapAll` each one wraps a
                // `Holder.Direct`, so unwrapping is safe and yields the real node.
                if (f instanceof DensityFunctions.HolderHolder holder) {
                    return holder.function().value();
                }
                return f;
            }

            @Override
            public DensityFunction.NoiseHolder visitNoise(DensityFunction.NoiseHolder h) {
                // A *constant* seed for every noise, so the Rust side can build
                // identical instances without needing the registry key. This
                // deliberately isolates graph evaluation from seed derivation —
                // the latter is tracked separately as unverified.
                long seed = 0L;
                NormalNoise nn = NormalNoise.create(
                        new net.minecraft.world.level.levelgen.XoroshiroRandomSource(seed),
                        h.noiseData().value());
                return new DensityFunction.NoiseHolder(h.noiseData(), nn);
            }
        };

        // When a corpus path is supplied, emit only the graphs it names. Both
        // sides then iterate exactly the same set, so the comparison cannot drift.
        java.util.Set<String> wanted = null;
        if (args.length > 1) {
            wanted = new java.util.HashSet<>();
            for (String line : java.nio.file.Files.readAllLines(
                    java.nio.file.Path.of(args[1]))) {
                if (line.isBlank() || line.startsWith("#")) {
                    continue;
                }
                int tab = line.indexOf('\t');
                wanted.add(tab < 0 ? line : line.substring(0, tab));
            }
        }

        List<Map.Entry<String, DensityFunction>> initialised = new ArrayList<>();
        for (Map.Entry<String, DensityFunction> e : named) {
            initialised.add(Map.entry(e.getKey(), e.getValue().mapAll(init)));
        }
        named = initialised;

        if (mode.equals("encode")) {
            // The corpus is consumed by both sides, so emit vanilla's own encoding.
            for (Map.Entry<String, DensityFunction> e : named) {
                var encoded = DensityFunctions.DIRECT_CODEC.encodeStart(
                        com.mojang.serialization.JsonOps.INSTANCE, e.getValue());
                String json = encoded.getOrThrow(
                        msg -> new IllegalStateException(e.getKey() + ": " + msg)).toString();
                System.out.println(e.getKey() + "\t" + json);
            }
            return;
        }

        // mode == "eval": reference values for the same graphs the Rust side loads.
        for (Map.Entry<String, DensityFunction> e : named) {
            if (wanted != null && !wanted.contains(e.getKey())) {
                continue;
            }
            for (int i = 0; i < 24; i++) {
                final int bx = blockX(i);
                final int by = blockY(i);
                final int bz = blockZ(i);
                var ctx = new DensityFunction.FunctionContext() {
                    @Override public int blockX() { return bx; }
                    @Override public int blockY() { return by; }
                    @Override public int blockZ() { return bz; }
                };
                emit(e.getKey(), 0L, i, e.getValue().compute(ctx));
            }
        }
    }
}
