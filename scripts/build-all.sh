#!/usr/bin/env bash
#
# Builds everything and runs the standalone harness against the Rust core.
# This is the fast inner loop: no Minecraft, no NeoForge, no gradle. Seconds,
# not tens of minutes.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

JDK="${MCGEN_JDK:-}"
if [[ -z "$JDK" ]]; then
  JDK="$(find "$ROOT/.tools" -maxdepth 1 -type d -name 'jdk-21*' | head -1)"
fi
if [[ -z "$JDK" || ! -x "$JDK/bin/javac" ]]; then
  echo "error: no JDK 21 found. Run scripts/fetch-jdk.sh first." >&2
  exit 1
fi

echo "==> [1/3] Rust unit tests"
(cd "$ROOT/rust" && cargo test --quiet)

echo
echo "==> [2/3] Rust release build"
(cd "$ROOT/rust" && cargo build --release --quiet)

echo
echo "==> [3/3] Java FFM harness (Java 21 preview API)"
HARNESS="$ROOT/java/harness"
mkdir -p "$HARNESS/out"
"$JDK/bin/javac" --release 21 --enable-preview \
  -d "$HARNESS/out" "$HARNESS/McGenHarness.java" 2>&1 | grep -v 'preview features\|Recompile' || true

exec "$JDK/bin/java" --enable-preview --enable-native-access=ALL-UNNAMED \
  -cp "$HARNESS/out" McGenHarness "$ROOT/rust/target/release/libmcgen.so"
