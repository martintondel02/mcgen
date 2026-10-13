#!/usr/bin/env bash
#
# Parity check: Rust vs the REAL Minecraft 1.21.1 classes.
#
# The oracle (tools/oracle/VanillaOracle.java) calls the actual Mojang code
# shipped in the NeoForge server install, so this compares against ground truth
# rather than against a reimplementation of ground truth.
#
# Compares raw f64 bit patterns, so a single differing mantissa bit fails.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
JDK="${MCGEN_JDK:-$(find "$ROOT/.tools" -maxdepth 1 -type d -name 'jdk-21*' | head -1)}"
SERVER="$ROOT/server"
MCJAR="$(find "$SERVER/libraries/net/minecraft/server" -name '*srg.jar' | head -1)"

if [[ -z "$MCJAR" ]]; then
  echo "error: Minecraft server jar not found. Run scripts/setup-server.sh first." >&2
  exit 1
fi

# shellcheck source=scripts/lib-classpath.sh
source "$ROOT/scripts/lib-classpath.sh"

# NOTE: the vanilla jar ships obfuscated single-letter classes, including a
# top-level `com.class`. If the intermediate `-unpacked`/`-slim` jars are on
# the classpath, that class shadows the entire `com.*` package and javac
# reports the bizarre error "cannot find symbol: variable mojang".
#
# The classpath is built by `mcgen_classpath`, which keeps the newest version of
# each artifact. That matters: this tree ships two guavas, and picking the wrong
# one makes the oracle die with NoSuchMethodError on a different machine.
CP="$MCJAR:$(mcgen_classpath "$SERVER/libraries" '*-unpacked.jar' '*-slim.jar')"
KINDS=("${@:-improved_noise}")
# `column` is in the list deliberately. It is the only check that walks all 256 columns of a
# chunk and compares against vanilla's own `NoiseChunk`, and on the round it was added a change
# that the other eleven all passed still moved 620 of its 98,304 values. A check that catches a
# real regression does not belong outside CI.
if [[ $# -eq 0 ]]; then
  KINDS=(improved_noise xoroshiro normal_noise hash math density density_noise \
         density_json density_dense density_router noise_seed column)
fi

echo "==> oracle jar: $(basename "$MCJAR")"

ORACLE="$ROOT/tools/oracle"
mkdir -p "$ORACLE/out"
# Do not swallow compile errors: under `set -e` a silent failure here makes the
# whole script exit with no output at all, which is genuinely hard to diagnose.
if ! "$JDK/bin/javac" -nowarn -cp "$CP" -d "$ORACLE/out" \
    "$ORACLE/VanillaOracle.java" "$ORACLE/RouterOracle.java"; then
  echo "error: could not compile the oracle. Fix it before comparing." >&2
  exit 1
fi

# The Minecraft classpath writes log noise to stdout (a log4j banner, and
# registry/boot warnings from static initialisers). Log lines always carry a
# "[timestamp] [thread/LEVEL]" prefix; data lines never contain "] [".
filter() { grep -vE '(^WARN |\] \[)' ; }

total_fail=0

for kind in "${KINDS[@]}"; do
  echo
  echo "########## $kind ##########"

  # Some kinds take an extra argument (a shared JSON corpus), and one is served
  # by a different oracle program entirely.
  extra=()
  java_main=VanillaOracle
  java_extra=()
  # Exported to the Rust side only for the kinds that need them.
  unset MCGEN_NOISE_NAMES MCGEN_ROUTER_JSON || true
  case "$kind" in
    noise_seed|column)
      # Chunk-level parity. Needs the router corpus plus the noise names, which the
      # JSON codec drops but `RandomState` derives the seed from.
      MCGEN_NOISE_NAMES="$ROOT/tools/oracle/cases/names.txt"
      MCGEN_ROUTER_JSON="$ROOT/tools/oracle/cases/router.jsonl"
      MCGEN_CLAMPS="$ROOT/tools/oracle/cases/clamps.txt"
      export MCGEN_NOISE_NAMES MCGEN_ROUTER_JSON MCGEN_CLAMPS
      java_main=RouterOracle
      java_extra=(-Dmcgen.names="$MCGEN_NOISE_NAMES" -Dmcgen.clamps="$MCGEN_CLAMPS")
      if [[ "$kind" == "noise_seed" ]]; then
        java_extra+=(-Dmcgen.only=noise_seed)
      fi
      ;;
  esac
  case "$kind" in
    density_json|density_dense)
      extra=("$ROOT/tools/oracle/cases/density.jsonl")
      ;;
    density_router)
      # The real overworld router, serialised by RouterOracle from vanilla's own
      # data generators.
      extra=("$ROOT/tools/oracle/cases/router.jsonl")
      java_main=RouterOracle
      ;;
  esac

  # The oracle writes data to its own file, so Minecraft's log noise on stdout
  # cannot contaminate the comparison.
  raw="/tmp/parity_java_${kind}_raw.txt"
  # RouterOracle takes a mode ("eval") rather than a kind.
  java_args=("$kind")
  if [[ "$java_main" == "RouterOracle" ]]; then
    java_args=(eval)
  fi
  if [[ "$kind" == "column" || "$kind" == "noise_seed" ]]; then
    java_args=(column)
  fi
  if ! "$JDK/bin/java" -Dmcgen.out="$raw" "${java_extra[@]}" -cp "$ORACLE/out:$CP" \
      "$java_main" "${java_args[@]}" "${extra[@]}" > /dev/null 2>&1; then
    echo "  FAIL: the Java oracle errored (rerun without 2>/dev/null to see why)"
    total_fail=$((total_fail + 1))
    continue
  fi
  filter < "$raw" > "/tmp/parity_java_$kind.txt"

  if ! (cd "$ROOT/rust" && cargo run --release --quiet --example parity_dump -- \
        "$kind" "${extra[@]}") \
      > "/tmp/parity_rust_$kind.txt"; then
    echo "  FAIL: the Rust side errored (see above)"
    total_fail=$((total_fail + 1))
    continue
  fi


  # `noise_value_rs` compares the noises `RandomState` wired against the parameters the graph
  # carries. Those differ on purpose — `RandomState` replaces some parameter sets as it wires —
  # so the comparison was never well-founded and is excluded rather than asserted. Everything
  # else in `column`, including all 98,304 chunked values, is compared as-is.
  if [[ "$kind" == column ]]; then
    for f in "/tmp/parity_rust_$kind.txt" "/tmp/parity_java_$kind.txt"; do
      grep -v '^noise_value_rs ' "$f" > "$f.tmp" && mv "$f.tmp" "$f"
    done
  fi

  rust_n=$(wc -l < "/tmp/parity_rust_$kind.txt")
  java_n=$(wc -l < "/tmp/parity_java_$kind.txt")

  if [[ "$rust_n" != "$java_n" ]]; then
    echo "  line count mismatch: rust=$rust_n java=$java_n"
    total_fail=$((total_fail + 1))
    continue
  fi

  if diff -q "/tmp/parity_rust_$kind.txt" "/tmp/parity_java_$kind.txt" >/dev/null; then
    echo "  PASS: $java_n/$java_n values bit-identical to real Minecraft"
  else
    bad=$(diff "/tmp/parity_rust_$kind.txt" "/tmp/parity_java_$kind.txt" | grep -c '^[<>]' || true)
    echo "  FAIL: $bad differing lines of $java_n"
    echo "  first differences (rust | java):"
    diff "/tmp/parity_rust_$kind.txt" "/tmp/parity_java_$kind.txt" | head -8 | sed 's/^/    /'
    total_fail=$((total_fail + 1))
  fi
done

echo
if [[ $total_fail -eq 0 ]]; then
  echo "=== PARITY OK: Rust matches Minecraft 1.21.1 bit for bit ==="
else
  echo "=== PARITY FAILURES in $total_fail kind(s) ===" >&2
  exit 1
fi
