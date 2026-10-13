# Installing mcgen on a server

## Requirements

| | |
|---|---|
| Minecraft | 1.21.1 |
| NeoForge | 21.1.257 (anything in the 21.1.x line should work) |
| Java | 21 |
| Architecture | 64-bit **Linux**, x86-64, glibc **2.34+** (Ubuntu 22.04, Debian 12, RHEL 9 or newer). See [Other platforms](#other-platforms). |
| Build tools | Only if you are building it yourself — see [BUILDING.md](BUILDING.md) |

You do **not** need `--enable-preview` or any JVM flags. That is precisely why the mod
ships a JNI shim rather than using Java 21's foreign-function API, which is still a
preview feature in 21.

## Install a release

Two files go to two places:

```
mcgen-0.2.0.jar      ->  <server>/mods/
libmcgenjni.so       ->  <server>/mcgen/
```

Create the `mcgen/` directory if it does not exist. Then start the server.

### Why the native library is not inside the jar

It is tempting to put `libmcgenjni.so` inside the jar and extract it at runtime. That is
how mods end up with path-traversal advisories and with a library written to a temp
directory that a second process can replace. Shipping it as a loose file next to the jar
is boring, inspectable, and lets you `ldd` it before you trust it.

## Verify it loaded

At startup you should see four lines:

```
[mcgen] loading /path/to/server/mcgen/libmcgenjni.so
[mcgen] ABI OK (v2), native library live inside NeoForge
[mcgen] boundary: 1,572,864 bytes, rust=0x7ee6ba2f40f8c1c8 java=0x7ee6ba2f40f8c1c8 MATCH
[mcgen] commands registered: /mcgen
```

The boundary line is a data-integrity check, not a correctness check: Rust hashes a
region, Java hash-checks the same bytes after they crossed JNI, and the two must agree.
It proves the bytes arrived intact. It says nothing about whether the terrain is right —
`/mcgen chunk` is what checks that.

## If something goes wrong

The mod is written so that **no failure stops your server booting**. If the library is
missing or incompatible, you get a message and everything else continues normally.

### `native library not found`

```
[mcgen] native library not found (set -Dmcgen.library=<path> or put it in ./mcgen/).
```

The mod looks, in order:

1. the `mcgen.library` system property,
2. `mcgen/libmcgenjni.so` relative to the server's working directory.

Set the property explicitly if your layout differs — add to `user_jvm_args.txt`:

```
-Dmcgen.library=/absolute/path/to/libmcgenjni.so
```

### `ABI mismatch`

```
[mcgen] ABI mismatch: library=1 mod=2 — skipping.
```

The jar and the `.so` are from different releases. Replace both together; they are
versioned as a pair.

### `commands registered` is missing

Command registration failed, and the line after it says why. The engine is still usable —
the startup self-test results are in the log either way.

### `/mcgen` is not recognised

The mod did not load at all. Check `mods/` contains the jar and that the log mentions
`mcgen` during NeoForge's mod loading.

## About the glibc requirement

`libmcgenjni.so` links only against `libm`, `libgcc_s` and `libc`. Its strongest
versioned symbol requirement is **GLIBC_2.34**.

It also carries *weak* references to `pidfd_spawnp` and `pidfd_getpid`, which are
GLIBC_2.39. Those are weak on purpose: Rust's standard library probes for them at load
time and falls back if they are absent, and this library never spawns a process anyway.
Do not be alarmed if `objdump` reports 2.39 — check the binding column first.

On a system older than glibc 2.34, build from source; see [BUILDING.md](BUILDING.md).

## Other platforms

The Rust crate is portable; the **build** currently targets Linux. To build for macOS or
Windows:

- Build the static library with `cargo build --release` (produces `libmcgen.a` on Unix,
  `mcgen.lib` on Windows).
- Compile `jni_shim.c` against your platform's `jni.h` and link the archive.
- The mod looks for `libmcgenjni.dylib` and `mcgenjni.dll` as well as the `.so`, so no
  Java change is needed.

This path is untested — the same honesty applies here as everywhere else in this project.
If you get it working, a PR that adds the build step would be welcome.

## Uninstall

Remove the jar from `mods/`. The `mcgen/` directory can stay or go; nothing else is
written anywhere. The mod has no config file and stores no state.
