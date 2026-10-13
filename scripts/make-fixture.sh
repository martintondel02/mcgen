#!/usr/bin/env bash
# Assemble the FFI's world fixture from the oracle's own output.
#
# The library used to build its world from a hand-written demo graph, which meant the in-server
# benchmark measured a toy rather than worldgen. This rebuilds it from the real overworld
# `final_density`, with the noise occurrences and clamp bounds the oracle reports for it — the
# same three things `parity_dump` needs to parse the graph faithfully.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/rust/mcgen-ffi/fixture/overworld.txt"
mkdir -p "$(dirname "$OUT")"

ROUTE=router_final_density
DENSITY="$(grep -m1 "^$ROUTE	" "$ROOT/tools/oracle/cases/router.jsonl" | cut -f2-)"
test -n "$DENSITY" || { echo "no $ROUTE in router.jsonl" >&2; exit 1; }

{
  echo "# mcgen fixture: the real overworld router, as the oracle sees it."
  echo "# density / occurrences / clamps, in the order the parser consumes them."
  echo "density	$DENSITY"
  echo "occurrences"
  awk -v r="$ROUTE" '$1==r {$1=""; sub(/^ /,""); print}' "$ROOT/tools/oracle/cases/names.txt"
  echo "clamps"
  awk -v r="$ROUTE" '$1==r {$1=""; sub(/^ /,""); print}' "$ROOT/tools/oracle/cases/clamps.txt"
} > "$OUT"
echo "wrote $OUT ($(wc -c < "$OUT") bytes, $(grep -c '' "$OUT") lines)"
echo
echo "Rebuild the library after regenerating, since the fixture is embedded with include_str!:"
echo "  (cd rust && cargo build --release)"
