#!/usr/bin/env bash
#
# Boots the NeoForge server with the mod installed and stops it once the mod's
# self-test has reported. Prints the mcgen lines.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SERVER="$ROOT/server"

JDK="${MCGEN_JDK:-}"
if [[ -z "$JDK" ]]; then
  JDK="$(find "$ROOT/.tools" -maxdepth 1 -type d -name 'jdk-21*' | head -1)"
fi
if [[ -z "$JDK" || ! -x "$JDK/bin/java" ]]; then
  echo "error: no JDK 21 found. Run scripts/fetch-jdk.sh first." >&2
  exit 1
fi
if [[ ! -f "$SERVER/mods/mcgen-0.2.0.jar" ]]; then
  echo "error: mod not staged. Run scripts/setup-server.sh first." >&2
  exit 1
fi

LOG="$SERVER/console.log"
: > "$LOG"

echo "==> starting NeoForge server (log: $LOG)"
cd "$SERVER"
"$JDK/bin/java" @user_jvm_args.txt \
  "@libraries/net/neoforged/neoforge/21.1.257/unix_args.txt" nogui \
  >> "$LOG" 2>&1 &
SERVER_PID=$!

cleanup() {
  if kill -0 "$SERVER_PID" 2>/dev/null; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT

for _ in $(seq 1 120); do
  sleep 2
  if grep -qa 'RESULT' "$LOG" 2>/dev/null; then break; fi
  if ! kill -0 "$SERVER_PID" 2>/dev/null; then break; fi
done

echo
echo "=== mcgen self-test output ==="
grep -a 'mcgen\]' "$LOG" || echo "(no mcgen output — check $LOG)"

echo
echo "=== server status ==="
grep -aE 'Done \(|Starting minecraft server|NeoForge mod loading' "$LOG" | tail -5 || true

if grep -qa 'mcgen\] RESULT' "$LOG"; then
  echo
  echo "PASS: the Rust library ran inside NeoForge."
  exit 0
fi

echo
echo "FAIL: no RESULT line. Last 30 log lines:" >&2
tail -30 "$LOG" >&2
exit 1
