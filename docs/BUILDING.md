# Building from source

## One-shot

```bash
./scripts/fetch-jdk.sh      # JDK 21 into .tools/
./scripts/setup-server.sh   # NeoForge server, mod jar, native library
```

That leaves you with:

```
build/mod/mcgen-0.2.0.jar
build/mod/libmcgenjni.so
```

## What each script does

| Script | Purpose |
|---|---|
| `fetch-jdk.sh` | Downloads a JDK 21 into `.tools/`. NeoForge 1.21.1 needs 21; a system JDK 25 will not work. |
| `setup-server.sh` | Runs the official NeoForge installer into `server/`, then builds and stages the mod. Idempotent. |
| `build-mod.sh` | Rust → static library → JNI shim → mod classes → jar. The fast inner loop. |
| `build-all.sh` | Everything, including the FFM harness. |
| `check-parity.sh` | The twelve parity checks against real Minecraft. |
| `test-commands.sh` | Boots the server and drives every `/mcgen` command through its console. |
| `make-fixture.sh` | Regenerates the embedded overworld router from the oracle. |
| `run-server.sh` | Boots the server and prints the mod's self-test output. |

## Why the mod compiles against real jars

`build-mod.sh` compiles the Java against **every jar in `server/libraries`** — the real
NeoForge, the real Minecraft, the real Brigadier — rather than against hand-written stubs.

An earlier version of this project used stubs, and it was fine while the mod touched
exactly two NeoForge types. The `/mcgen` tree needs `RegisterCommandsEvent`, `IEventBus`,
`CommandSourceStack`, `LevelChunk`, `BlockState` and more. A stub that is subtly wrong
compiles perfectly and fails at runtime with `NoSuchMethodError`, and that is a miserable
class of bug to debug. Compiling against the shipping jars makes it impossible.

The trade-off is that `setup-server.sh` must run first. That is a reasonable price, and
the build says so clearly if `server/libraries` is missing.

## The build is checked, not trusted

`build-mod.sh` fails if:

- the JNI shim still has a `DT_NEEDED` entry for `libmcgen.so` (the static link silently
  fell back to dynamic),
- any `net/minecraft`, `net/neoforged` or `com/mojang` class ended up inside the jar
  (the classpath leaked into the artifact, which would be a licensing problem).

## Regenerating the fixture

`rust/mcgen-ffi/fixture/overworld.txt` is generated from the oracle:

```bash
./scripts/make-fixture.sh
```

It is embedded with `include_str!`, so **rebuild Rust after changing it**. Forgetting
this silently produces stale results, which is why the script prints a reminder.

## Tests

```bash
cargo test --manifest-path rust/Cargo.toml     # 66 tests
./scripts/check-parity.sh                      # the real check
./scripts/test-commands.sh                     # end to end, in a live server
```

The Rust unit tests cover parsing, arithmetic and the caches. They are necessary and not
sufficient: the parity suite is what actually establishes correctness, and
`test-commands.sh` is what establishes that it works in a server.

## Environment variables

| Variable | Effect |
|---|---|
| `MCGEN_JDK` | Use a specific JDK instead of searching `.tools/` |
| `MCGEN_FRESH_WORLD=1` | Delete `server/world` so it regenerates — needed after changing `level-type` or the seed, because a stale world makes every `/mcgen chunk` comparison meaningless |
| `MCGEN_ROUTER_JSON`, `MCGEN_NOISE_NAMES`, `MCGEN_CLAMPS`, `MCGEN_STATS`, `MCGEN_OUT` | Drive the oracle and its diagnostics |
| `-Dmcgen.library=<path>` | Where the mod looks for the native library |
| `-Dmcgen.benchChunks=<n>` | Side length of the startup benchmark region (default 8) |
