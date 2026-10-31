#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
export PATH="$SCRIPT_DIR/.toolchain/bin:$PATH"

echo "==> Compilando ChronoKeep em modo release para Windows (x86_64-pc-windows-msvc)..."
cargo build --manifest-path "$SCRIPT_DIR/rust-workspace/Cargo.toml" --workspace --target x86_64-pc-windows-msvc --release

echo "==> Atualizando binarios embutidos no instalador..."
cp "$SCRIPT_DIR/rust-workspace/target/x86_64-pc-windows-msvc/release/chrono-daemon.exe" "$SCRIPT_DIR/rust-workspace/crates/cr-gui/embedded/chrono-daemon.exe"
cp "$SCRIPT_DIR/rust-workspace/target/x86_64-pc-windows-msvc/release/chronokeep.dll" "$SCRIPT_DIR/rust-workspace/crates/cr-gui/embedded/chronokeep.dll"

echo "==> Recompilando ChronoKeep com binarios atualizados..."
cargo build --manifest-path "$SCRIPT_DIR/rust-workspace/Cargo.toml" -p cr-gui --target x86_64-pc-windows-msvc --release

echo "==> Copiando binarios para a pasta dist/..."
mkdir -p "$SCRIPT_DIR/dist"
cp "$SCRIPT_DIR/rust-workspace/target/x86_64-pc-windows-msvc/release/ChronoKeep.exe" "$SCRIPT_DIR/dist/ChronoKeep.exe"
cp "$SCRIPT_DIR/rust-workspace/target/x86_64-pc-windows-msvc/release/ChronoKeep.exe" "$SCRIPT_DIR/dist/ChronoKeepSetup.exe"
cp "$SCRIPT_DIR/rust-workspace/target/x86_64-pc-windows-msvc/release/chrono-daemon.exe" "$SCRIPT_DIR/dist/chrono-daemon.exe"
cp "$SCRIPT_DIR/rust-workspace/target/x86_64-pc-windows-msvc/release/chronokeep.dll" "$SCRIPT_DIR/dist/chronokeep.dll"
cp "$SCRIPT_DIR/rust-workspace/target/x86_64-pc-windows-msvc/release/chronokeep.dll" "$SCRIPT_DIR/dist/cloud_redirect.dll"

echo "==> Concluido com sucesso! Binarios atualizados em dist/:"
ls -lh "$SCRIPT_DIR/dist/"*.exe "$SCRIPT_DIR/dist/"*.dll
