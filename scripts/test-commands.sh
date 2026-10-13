#!/usr/bin/env bash
#
# Boots the NeoForge server and drives the /mcgen command tree through its console,
# then prints everything the mod said. This is the end-to-end test: it exercises the
# real NeoForge event bus, the real command dispatcher and the real JNI boundary —
# nothing here is mocked.
#
# Usage: scripts/test-commands.sh [chunkX chunkZ ...]
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

FIFO="$SERVER/.console.in"
rm -f "$FIFO"
mkfifo "$FIFO"

echo "==> starting NeoForge server (log: $LOG)"
cd "$SERVER"
"$JDK/bin/java" @user_jvm_args.txt \
  "@libraries/net/neoforged/neoforge/21.1.257/unix_args.txt" nogui \
  < "$FIFO" >> "$LOG" 2>&1 &
SERVER_PID=$!

# Hold the write end open so the server's stdin never sees EOF and shuts down.
exec 3>"$FIFO"

cleanup() {
  exec 3>&- 2>/dev/null || true
  if kill -0 "$SERVER_PID" 2>/dev/null; then
    kill "$SERVER_PID" 2>/dev/null || true
    wait "$SERVER_PID" 2>/dev/null || true
  fi
  rm -f "$FIFO"
}
trap cleanup EXIT

echo "==> waiting for the server to finish starting"
READY=0
for _ in $(seq 1 180); do
  sleep 2
  if grep -qaE 'Done \(' "$LOG" 2>/dev/null; then READY=1; break; fi
  if ! kill -0 "$SERVER_PID" 2>/dev/null; then break; fi
done

if [[ "$READY" != "1" ]]; then
  echo "error: the server did not reach 'Done'. Last 40 lines:" >&2
  tail -40 "$LOG" >&2
  exit 1
fi
echo "==> server is up; driving commands"

send() {
  echo "$1" >&3
  sleep "${2:-3}"
}

send "mcgen status" 3
send "mcgen noises" 3
send "mcgen verify" 5
send "mcgen bench 2" 8
send "mcgen density 0 64 0" 3

# Compare the engine against real vanilla terrain. Chunk 0,0 is inside the
# spawn area, so it is loaded without this command generating anything.
if [[ $# -ge 2 ]]; then
  send "mcgen chunk $1 $2" 10
else
  send "mcgen chunk 0 0" 10
  send "mcgen chunk 1 0" 10
fi

sleep 2
echo
echo "=================== /mcgen output ==================="
grep -a 'mcgen\]' "$LOG" | sed 's/^.*\[mcgen\] /[mcgen] /'
echo "===================================================="

if grep -qaE 'Unknown or incomplete command|Unknown command' "$LOG"; then
  echo
  echo "ERROR: the server did not recognise a command." >&2
  exit 1
fi

echo
if grep -qa 'mcgen\] chunk' "$LOG" && grep -qaE 'positions agree \(100\.000%\)' "$LOG"; then
  echo "PASS: /mcgen commands work, and the engine matched vanilla terrain exactly."
  exit 0
fi
echo "PARTIAL: commands ran; see the output above for what matched."
