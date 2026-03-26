#!/usr/bin/env sh
# Sentrix CLI installer — auto-detects platform and architecture.
# Usage: curl -fsSL https://raw.githubusercontent.com/ch4r10t33r/sentrix/main/install.sh | sh
set -e

REPO="ch4r10t33r/sentrix"
BIN="sentrix"
INSTALL_DIR="${SENTRIX_INSTALL_DIR:-/usr/local/bin}"

# ── Detect OS ────────────────────────────────────────────────────────────────
OS=$(uname -s 2>/dev/null || echo "unknown")
case "$OS" in
    Darwin) OS="darwin" ;;
    Linux)  OS="linux"  ;;
    *)
        echo "Unsupported OS: $OS"
        echo "Install via npm instead: npm install -g @ch4r10teer41/sentrix-cli"
        exit 1
        ;;
esac

# ── Detect arch ──────────────────────────────────────────────────────────────
ARCH=$(uname -m 2>/dev/null || echo "unknown")
case "$ARCH" in
    x86_64)        ARCH="x64"   ;;
    aarch64|arm64) ARCH="arm64" ;;
    *)
        echo "Unsupported architecture: $ARCH"
        echo "Install via npm instead: npm install -g @ch4r10teer41/sentrix-cli"
        exit 1
        ;;
esac

ASSET="${BIN}-${OS}-${ARCH}"
URL="https://github.com/${REPO}/releases/latest/download/${ASSET}"

echo "Downloading sentrix (${OS}/${ARCH})..."
curl -fsSL "$URL" -o "/tmp/${BIN}"
chmod +x "/tmp/${BIN}"

# ── Install ──────────────────────────────────────────────────────────────────
if [ -w "$INSTALL_DIR" ]; then
    mv "/tmp/${BIN}" "${INSTALL_DIR}/${BIN}"
else
    printf "sudo required to write to %s — enter password if prompted\n" "$INSTALL_DIR"
    sudo mv "/tmp/${BIN}" "${INSTALL_DIR}/${BIN}"
fi

echo ""
echo "sentrix installed to ${INSTALL_DIR}/${BIN}"
"${INSTALL_DIR}/${BIN}" --version
