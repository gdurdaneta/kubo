#!/usr/bin/env bash
# Compila un binario portable en un contenedor con glibc viejo.
#
# Compilar en Pop!_OS 24.04 produce un binario que pide GLIBC 2.39 (por
# `pidfd_spawnp`, que trae std al usar `Command`), y eso deja afuera a
# cualquiera en Ubuntu 22.04. glibc es compatible hacia adelante, no hacia
# atrás: hay que compilar contra el más viejo al que se quiera llegar.
set -euo pipefail

BASE="${BASE:-ubuntu:22.04}"          # glibc 2.35
SALIDA="${SALIDA:-dist}"
NOMBRE=kubo

mkdir -p "$SALIDA"
docker run --rm \
  -v "$PWD":/src -w /src \
  -v "$PWD/.cargo-docker":/usr/local/cargo/registry \
  -e CARGO_TARGET_DIR=/src/target-dist \
  "$BASE" bash -euo pipefail -c '
    export DEBIAN_FRONTEND=noninteractive
    apt-get update -qq
    apt-get install -y -qq --no-install-recommends curl ca-certificates build-essential pkg-config >/dev/null
    curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable >/dev/null
    . "$HOME/.cargo/env"
    cargo build --release
  '

cp "target-dist/release/$NOMBRE" "$SALIDA/$NOMBRE"
strip "$SALIDA/$NOMBRE" 2>/dev/null || true

echo
echo "binario: $SALIDA/$NOMBRE  ($(du -h "$SALIDA/$NOMBRE" | cut -f1))"
echo "glibc mínimo: $(objdump -T "$SALIDA/$NOMBRE" | grep -oE 'GLIBC_[0-9.]+' | sort -V -u | tail -1)"
echo "enlaza contra:"
objdump -p "$SALIDA/$NOMBRE" | awk '/NEEDED/{print "  " $2}'
