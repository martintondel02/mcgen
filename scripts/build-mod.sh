#!/usr/bin/env bash
#
# Builds the NeoForge mod jar:
#   Rust staticlib -> JNI shim -> mod classes -> mcgen-<version>.jar
#
# Compiles against the REAL NeoForge and Minecraft jars that scripts/setup-server.sh
# downloaded, rather than against hand-written stubs. The stub approach worked while
# the mod touched exactly two NeoForge types, but the /mcgen command tree needs
# RegisterCommandsEvent, IEventBus, CommandSourceStack, LevelChunk and more — and a
# stub that is subtly wrong compiles fine and fails at runtime with NoSuchMethodError.
# Compiling against the shipping jars makes that class of bug impossible.
#
# The Minecraft and NeoForge jars are never packaged: they are already on the
# server's classpath, and redistributing them would be a licensing problem.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MOD="$ROOT/java/neoforge-mod"
OUT="$ROOT/build/mod"
VERSION="0.2.0"

# Locate the JDK 21 we downloaded (NeoForge 1.21.1 needs 21, not the system's 25).
JDK="${MCGEN_JDK:-}"
if [[ -z "$JDK" ]]; then
  JDK="$(find "$ROOT/.tools" -maxdepth 1 -type d -name 'jdk-21*' | head -1)"
fi
if [[ -z "$JDK" || ! -x "$JDK/bin/javac" ]]; then
  echo "error: no JDK 21 found. Run scripts/fetch-jdk.sh first, or set MCGEN_JDK." >&2
  exit 1
fi

LIBS="$ROOT/server/libraries"
if [[ ! -d "$LIBS" ]]; then
  echo "error: $LIBS is missing." >&2
  echo "       Run scripts/setup-server.sh first — the mod compiles against the real" >&2
  echo "       NeoForge and Minecraft jars, which that script downloads." >&2
  exit 1
fi

echo "==> JDK:    $JDK"
echo "==> output: $OUT"

rm -rf "$OUT"
mkdir -p "$OUT/classes"

# Every library the server ships, keeping the newest version of each artifact.
# Over-broad on purpose: pinning the exact subset would be one more thing to get
# wrong, and javac only reads what it is asked for. The dedupe is not optional —
# this tree ships guava 20 and 32, and compiling against 20 produces classes that
# fail at runtime against 32.
# shellcheck source=scripts/lib-classpath.sh
source "$ROOT/scripts/lib-classpath.sh"
CP="$(mcgen_classpath "$LIBS")"
if [[ -z "$CP" ]]; then
  echo "error: no jars under $LIBS" >&2
  exit 1
fi

echo "==> [1/5] building Rust (cdylib + staticlib)"
(cd "$ROOT/rust" && cargo build --release --quiet)
ls -l "$ROOT/rust/target/release/libmcgen.a" "$ROOT/rust/target/release/libmcgen.so" >/dev/null

echo "==> [2/5] compiling JNI shim"
# Link the Rust *static* archive by full path. Using -lmcgen would let the
# linker pick libmcgen.so (the cdylib built for the FFM harness), which leaves
# the shim with an unsatisfied DT_NEEDED entry at runtime.
# -Wl,-s strips debug info. The Rust static archive is built with debug symbols,
# which makes the linked shim 6.5 MB; stripped it is 2.1 MB, and it is a release
# artefact that a server loads on every start. The runtime dependencies are only
# libm, libgcc_s and libc, so it stays portable.
cc -shared -fPIC -O2 -Wall -Wextra \
  -I"$JDK/include" -I"$JDK/include/linux" \
  -Wl,-s \
  -o "$OUT/libmcgenjni.so" \
  "$MOD/native/jni_shim.c" \
  "$ROOT/rust/target/release/libmcgen.a" \
  -lpthread -ldl -lm
echo "    $(ls -l "$OUT/libmcgenjni.so" | awk '{print $5" bytes"}')"

# Fail the build if the shim still depends on the cdylib.
if command -v readelf >/dev/null 2>&1; then
  if readelf -d "$OUT/libmcgenjni.so" | grep -q 'libmcgen\.so'; then
    echo "error: JNI shim still depends on libmcgen.so; static link failed" >&2
    exit 1
  fi
  echo "    self-contained (no libmcgen.so dependency)"
fi

echo "==> [3/5] compiling mod against the real NeoForge + Minecraft jars"
"$JDK/bin/javac" --release 21 -Xlint:all -Xlint:-processing \
  -cp "$CP" -d "$OUT/classes" $(find "$MOD/src" -name '*.java')

echo "==> [4/5] copying resources"
cp -r "$MOD/resources/." "$OUT/classes/"

echo "==> [5/5] packaging jar"
(cd "$OUT/classes" && "$JDK/bin/jar" --create --file "$OUT/mcgen-$VERSION.jar" .)

# The jar must contain only our own classes and resources. Anything from
# net/minecraft or net/neoforged means the classpath leaked into the artifact,
# which would both bloat it and risk a licensing problem.
if "$JDK/bin/jar" --list --file "$OUT/mcgen-$VERSION.jar" | grep -qE '^(net/minecraft|net/neoforged|com/mojang)/'; then
  echo "error: server classes leaked into the jar" >&2
  exit 1
fi

echo
echo "Built: $OUT/mcgen-$VERSION.jar"
"$JDK/bin/jar" --list --file "$OUT/mcgen-$VERSION.jar"
