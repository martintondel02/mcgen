#!/usr/bin/env bash
#
# Installs a NeoForge 21.1.257 (Minecraft 1.21.1) dedicated server and stages the
# mod into it. Idempotent: re-running only re-stages the artifacts.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SERVER="$ROOT/server"
CACHE="$ROOT/.cache"
NEOFORGE_VERSION="21.1.257"

JDK="${MCGEN_JDK:-}"
if [[ -z "$JDK" ]]; then
  JDK="$(find "$ROOT/.tools" -maxdepth 1 -type d -name 'jdk-21*' | head -1)"
fi
if [[ -z "$JDK" || ! -x "$JDK/bin/java" ]]; then
  echo "error: no JDK 21 found. Run scripts/fetch-jdk.sh first." >&2
  exit 1
fi

mkdir -p "$SERVER" "$CACHE"

INSTALLER="$CACHE/neoforge-$NEOFORGE_VERSION-installer.jar"

if [[ ! -f "$SERVER/libraries/net/neoforged/neoforge/$NEOFORGE_VERSION/unix_args.txt" ]]; then
  if [[ ! -f "$INSTALLER" ]]; then
    echo "==> downloading NeoForge $NEOFORGE_VERSION installer"
    curl -fSL --progress-bar -o "$INSTALLER" \
      "https://maven.neoforged.net/releases/net/neoforged/neoforge/$NEOFORGE_VERSION/neoforge-$NEOFORGE_VERSION-installer.jar"
  fi
  echo "==> installing the server (downloads the Minecraft server jar and libraries)"
  (cd "$SERVER" && "$JDK/bin/java" -jar "$INSTALLER" --installServer)
else
  echo "==> NeoForge $NEOFORGE_VERSION already installed"
fi

echo "==> building the mod"
"$ROOT/scripts/build-mod.sh" >/dev/null

echo "==> staging mod + native library"
mkdir -p "$SERVER/mods" "$SERVER/mcgen"
cp "$ROOT/build/mod/mcgen-0.2.0.jar" "$SERVER/mods/"
cp "$ROOT/build/mod/libmcgenjni.so" "$SERVER/mcgen/"

# NeoForge will not start without accepting the Minecraft EULA.
echo "eula=true" > "$SERVER/eula.txt"

# A normal world with a fixed seed, so /mcgen chunk can compare the engine against
# real vanilla terrain. The seed is pinned so results are reproducible, and the file
# is rewritten every time: this is a dedicated throwaway test server, and a stale
# level-type silently makes every /mcgen chunk comparison meaningless.
cat > "$SERVER/server.properties" <<'EOF'
level-type=minecraft:normal
level-seed=mcgen-demo
online-mode=false
spawn-protection=0
view-distance=6
simulation-distance=4
max-players=2
sync-chunk-writes=false
allow-nether=false
EOF
# The pinned seed is a string, so the numeric seed the engine needs is whatever the
# server derived from it — which is why the commands read it back from the level.

# A world generated under a different level-type or seed cannot be compared
# against, so allow the caller to force a fresh one.
if [[ "${MCGEN_FRESH_WORLD:-0}" == "1" ]]; then
  echo "==> MCGEN_FRESH_WORLD=1: removing $SERVER/world so it regenerates"
  rm -rf "$SERVER/world"
fi

if [[ ! -f "$SERVER/user_jvm_args.txt" ]] || ! grep -q -- '-Xmx' "$SERVER/user_jvm_args.txt"; then
  printf -- '-Xmx2G\n-Xms1G\n' > "$SERVER/user_jvm_args.txt"
fi

echo
echo "Ready. Start it with: scripts/run-server.sh"
