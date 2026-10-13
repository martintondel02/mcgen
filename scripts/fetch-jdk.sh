#!/usr/bin/env bash
#
# Downloads a JDK 21 into .tools/. Needed because NeoForge 1.21.1 targets
# Java 21, and the FFM API we use in the harness is a *preview* feature in 21
# (JEP 442) which only became final in Java 22 (JEP 454) — so a newer JDK is not
# a drop-in substitute for NeoForge's expected runtime.
#
# Uses a local tarball rather than a system package so no sudo is required.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TOOLS="$ROOT/.tools"
CACHE="$ROOT/.cache"
mkdir -p "$TOOLS" "$CACHE"

if find "$TOOLS" -maxdepth 1 -type d -name 'jdk-21*' | grep -q .; then
  echo "JDK 21 already present:"
  find "$TOOLS" -maxdepth 1 -type d -name 'jdk-21*'
  exit 0
fi

URL="https://api.adoptium.net/v3/binary/latest/21/ga/linux/x64/jdk/hotspot/normal/eclipse"
TARBALL="$CACHE/jdk21.tar.gz"

echo "==> downloading Temurin JDK 21"
curl -fSL --progress-bar -o "$TARBALL" "$URL"

echo "==> extracting to $TOOLS"
tar -xzf "$TARBALL" -C "$TOOLS"

JDK="$(find "$TOOLS" -maxdepth 1 -type d -name 'jdk-21*' | head -1)"
echo "==> installed: $JDK"
"$JDK/bin/javac" -version
