#!/bin/bash
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
if [ -d "$SCRIPT_DIR/.toolchain/bin" ]; then
  export PATH="$SCRIPT_DIR/.toolchain/bin:$HOME/.cargo/bin:$PATH"
elif [ -d "/home/ubuntu/projetos/cr-clone/.toolchain/bin" ]; then
  export PATH="/home/ubuntu/projetos/cr-clone/.toolchain/bin:$HOME/.cargo/bin:$PATH"
else
  export PATH="$HOME/.cargo/bin:$PATH"
fi

echo "==> Compilando ChronoKeep em modo release para Windows (x86_64-pc-windows-msvc)..."
cargo build --release -p cr-service -p cr-bridge-dll --target x86_64-pc-windows-msvc

echo "==> Empacotando e mascarando binarios embutidos (antivirus false-positive mitigation)..."
python3 "$SCRIPT_DIR/scripts/pack_embedded.py" "$SCRIPT_DIR/target/x86_64-pc-windows-msvc/release/chrono-daemon.exe" "$SCRIPT_DIR/crates/cr-gui/embedded/chrono-daemon.bin"
python3 "$SCRIPT_DIR/scripts/pack_embedded.py" "$SCRIPT_DIR/target/x86_64-pc-windows-msvc/release/chronokeep.dll" "$SCRIPT_DIR/crates/cr-gui/embedded/chronokeep.bin"
rm -f "$SCRIPT_DIR/crates/cr-gui/embedded/chrono-daemon.exe" "$SCRIPT_DIR/crates/cr-gui/embedded/chronokeep.dll"

echo "==> Recompilando ChronoKeep com binarios atualizados..."
cargo build --release -p cr-gui --target x86_64-pc-windows-msvc

echo "==> Copiando binarios para a pasta dist/..."
mkdir -p "$SCRIPT_DIR/dist"
rm -f "$SCRIPT_DIR/dist/ChronoKeepSetup.exe" "$SCRIPT_DIR/dist/ChronoKeepUninstall.exe"
cp "$SCRIPT_DIR/target/x86_64-pc-windows-msvc/release/ChronoKeep.exe" "$SCRIPT_DIR/dist/ChronoKeep.exe"
cp "$SCRIPT_DIR/target/x86_64-pc-windows-msvc/release/chrono-daemon.exe" "$SCRIPT_DIR/dist/chrono-daemon.exe"
cp "$SCRIPT_DIR/target/x86_64-pc-windows-msvc/release/chronokeep.dll" "$SCRIPT_DIR/dist/chronokeep.dll"
cp "$SCRIPT_DIR/target/x86_64-pc-windows-msvc/release/chronokeep.dll" "$SCRIPT_DIR/dist/cloud_redirect.dll"

echo "==> Concluido com sucesso! Binarios atualizados em dist/:"
ls -lh "$SCRIPT_DIR/dist/"*.exe "$SCRIPT_DIR/dist/"*.dll
