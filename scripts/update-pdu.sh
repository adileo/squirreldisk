#!/usr/bin/env bash
set -euo pipefail

PDU_VERSION="${1:-0.23.0}"
BASE_URL="https://github.com/KSXGitHub/parallel-disk-usage/releases/download/${PDU_VERSION}"
BIN_DIR="$(cd "$(dirname "$0")/../src-tauri/bin" && pwd)"

echo "Updating pdu to ${PDU_VERSION} in ${BIN_DIR}"

download() {
  local name="$1"
  local dest="$2"
  echo "  Downloading ${name}..."
  curl -fL "${BASE_URL}/${name}" -o "${dest}"
}

# Pre-built binaries available in the release
download "pdu-x86_64-apple-darwin"     "${BIN_DIR}/pdu-x86_64-apple-darwin"
download "pdu-x86_64-pc-windows-gnu.exe" "${BIN_DIR}/pdu-x86_64-pc-windows-msvc.exe"
download "pdu-x86_64-unknown-linux-gnu" "${BIN_DIR}/pdu-x86_64-unknown-linux-gnu"
chmod +x "${BIN_DIR}/pdu-x86_64-apple-darwin" "${BIN_DIR}/pdu-x86_64-unknown-linux-gnu"

# Build natively for aarch64-apple-darwin (no pre-built binary in releases)
echo "  Building pdu ${PDU_VERSION} natively for aarch64-apple-darwin..."
CARGO_TARGET_DIR="$(mktemp -d)"
cargo install "parallel-disk-usage@${PDU_VERSION}" \
  --target aarch64-apple-darwin \
  --root "${CARGO_TARGET_DIR}"
cp "${CARGO_TARGET_DIR}/bin/pdu" "${BIN_DIR}/pdu-aarch64-apple-darwin"
rm -rf "${CARGO_TARGET_DIR}"

echo "Done. pdu ${PDU_VERSION} installed to ${BIN_DIR}"
