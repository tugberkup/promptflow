#!/usr/bin/env bash
# Copies the runtime DLLs the Windows installer ships (see src-tauri/windows/installer-hooks.nsh) next to
# the release build, before `cargo tauri build`. DirectML.dll gets there on its own: ONNX Runtime's
# build copies it.
set -euo pipefail

out="${CARGO_TARGET_DIR:-$(dirname "$0")/../src-tauri/target}/release"
mkdir -p "$out"

vswhere="/c/Program Files (x86)/Microsoft Visual Studio/Installer/vswhere.exe"
crt=$("$vswhere" -latest -products '*' -find 'VC\Redist\MSVC\*\x64\Microsoft.VC143.CRT\msvcp140.dll' | head -n 1 | tr -d '\r')
if [ -z "$crt" ]; then
  echo "Visual C++ redistributable DLLs not found (install the C++ build tools)" >&2
  exit 1
fi
crt=$(dirname "$crt")
for dll in msvcp140.dll msvcp140_1.dll vcruntime140.dll vcruntime140_1.dll; do
  cp "$crt/$dll" "$out/"
done

# The Vulkan loader, installed with GPU drivers and the Vulkan runtime
cp /c/Windows/System32/vulkan-1.dll "$out/"
ls -l "$out"/*.dll
