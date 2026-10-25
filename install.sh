#!/usr/bin/env sh
# uc installer for Linux (x86_64, aarch64) and macOS (Apple Silicon arm64, Intel x86_64)
# Usage:
#   curl -LsSf https://raw.githubusercontent.com/omarespejel/uc/main/install.sh | sh
#   curl -LsSf https://raw.githubusercontent.com/omarespejel/uc/main/install.sh | sh -s -- --drop-in
set -eu

UC_REPO="${UC_REPO:-omarespejel/uc}"
UC_VERSION="${UC_VERSION:-latest}"
UC_INSTALL_DIR="${UC_INSTALL_DIR:-$HOME/.local/bin}"
UC_INSTALL_SYMLINKS="${UC_INSTALL_SYMLINKS:-0}"

for arg in "$@"; do
  case "$arg" in
    --drop-in|--symlinks)
      UC_INSTALL_SYMLINKS=1
      ;;
    --prefix=*)
      UC_INSTALL_DIR="${arg#*=}"
      ;;
  esac
done

detect_target() {
  os_name="$(uname -s)"
  arch_name="$(uname -m)"

  case "$os_name" in
    Linux)
      os_part="unknown-linux-gnu"
      ;;
    Darwin)
      os_part="apple-darwin"
      ;;
    *)
      echo "error: unsupported operating system: $os_name (supported: Linux, macOS)" >&2
      exit 1
      ;;
  esac

  case "$arch_name" in
    x86_64|amd64)
      arch_part="x86_64"
      ;;
    aarch64|arm64)
      arch_part="aarch64"
      ;;
    *)
      echo "error: unsupported CPU architecture: $arch_name (supported: x86_64, aarch64/arm64)" >&2
      exit 1
      ;;
  esac

  echo "${arch_part}-${os_part}"
}

TARGET="$(detect_target)"
mkdir -p "$UC_INSTALL_DIR"

echo "Installing uc for ${TARGET} into ${UC_INSTALL_DIR}..."

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" 2>/dev/null && pwd || pwd)"
if [ -x "${SCRIPT_DIR}/target/release/uc" ]; then
  cp "${SCRIPT_DIR}/target/release/uc" "${UC_INSTALL_DIR}/uc"
  chmod +x "${UC_INSTALL_DIR}/uc"
elif [ -f "${SCRIPT_DIR}/Cargo.toml" ] && command -v cargo >/dev/null 2>&1; then
  echo "Building self-contained uc binary from source..."
  (cd "$SCRIPT_DIR" && cargo build --release -p uc-cli)
  cp "${SCRIPT_DIR}/target/release/uc" "${UC_INSTALL_DIR}/uc"
  chmod +x "${UC_INSTALL_DIR}/uc"
else
  if [ "$UC_VERSION" = "latest" ]; then
    DOWNLOAD_URL="https://github.com/${UC_REPO}/releases/latest/download/uc-${TARGET}.tar.gz"
  else
    DOWNLOAD_URL="https://github.com/${UC_REPO}/releases/download/${UC_VERSION}/uc-${TARGET}.tar.gz"
  fi
  TMP_DIR="$(mktemp -d)"
  trap 'rm -rf "$TMP_DIR"' EXIT
  echo "Downloading ${DOWNLOAD_URL}..."
  curl -LsSf "$DOWNLOAD_URL" -o "${TMP_DIR}/uc.tar.gz"
  tar -xzf "${TMP_DIR}/uc.tar.gz" -C "$TMP_DIR"
  cp "${TMP_DIR}/uc" "${UC_INSTALL_DIR}/uc"
  chmod +x "${UC_INSTALL_DIR}/uc"
fi

if [ "$UC_INSTALL_SYMLINKS" = "1" ]; then
  for tool in scarb snforge sncast starknet-devnet; do
    ln -sf "${UC_INSTALL_DIR}/uc" "${UC_INSTALL_DIR}/${tool}"
    echo "  Linked ${UC_INSTALL_DIR}/${tool} -> uc"
  done
fi

echo "Successfully installed uc ($("${UC_INSTALL_DIR}/uc" --version 2>/dev/null || echo 'ready')) to ${UC_INSTALL_DIR}/uc"
case ":${PATH}:" in
  *":${UC_INSTALL_DIR}:"*) ;;
  *)
    echo "Add ${UC_INSTALL_DIR} to your PATH:"
    echo "  export PATH=\"${UC_INSTALL_DIR}:\$PATH\""
    ;;
esac
