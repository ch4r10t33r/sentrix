#!/usr/bin/env sh
# Borgkit CLI installer — auto-detects platform and architecture.
# Usage: curl -fsSL https://raw.githubusercontent.com/ch4r10t33r/borgkit/main/install.sh | sh
set -e

REPO="ch4r10t33r/borgkit"
BIN="borgkit"
INSTALL_DIR="${BORGKIT_INSTALL_DIR:-/usr/local/bin}"

# ── Detect OS ────────────────────────────────────────────────────────────────
OS=$(uname -s 2>/dev/null || echo "unknown")
case "$OS" in
    Darwin) OS="darwin" ;;
    Linux)  OS="linux"  ;;
    *)
        echo "Unsupported OS: $OS"
        echo "Install via npm instead: npm install -g @ch4r10teer41/borgkit-cli"
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
        echo "Install via npm instead: npm install -g @ch4r10teer41/borgkit-cli"
        exit 1
        ;;
esac

ASSET="${BIN}-${OS}-${ARCH}"
URL="https://github.com/${REPO}/releases/latest/download/${ASSET}"

echo "Downloading borgkit (${OS}/${ARCH})..."
curl -fsSL "$URL" -o "/tmp/${BIN}"
chmod +x "/tmp/${BIN}"

# ── Install ──────────────────────────────────────────────────────────────────
if [ -w "$INSTALL_DIR" ]; then
    mv "/tmp/${BIN}" "${INSTALL_DIR}/${BIN}"
elif command -v sudo >/dev/null 2>&1 && [ -t 0 ]; then
    # Interactive terminal — sudo is fine
    printf "sudo required to write to %s — enter password if prompted\n" "$INSTALL_DIR"
    sudo mv "/tmp/${BIN}" "${INSTALL_DIR}/${BIN}"
else
    # Non-interactive (piped shell) or no sudo — fall back to ~/bin
    INSTALL_DIR="$HOME/.local/bin"
    mkdir -p "$INSTALL_DIR"
    mv "/tmp/${BIN}" "${INSTALL_DIR}/${BIN}"
    printf "\nInstalled to %s (not on PATH via sudo — add it to your PATH):\n" "$INSTALL_DIR"
    printf '  echo '"'"'export PATH="$HOME/.local/bin:$PATH"'"'"' >> ~/.zshrc && source ~/.zshrc\n'
fi

echo ""
echo "borgkit installed to ${INSTALL_DIR}/${BIN}"
"${INSTALL_DIR}/${BIN}" --version
