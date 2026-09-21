#!/bin/sh
# Install the latest JAMA release binary for this platform.
#
#   curl -fsSL https://raw.githubusercontent.com/Mimf-India/jama/main/install.sh | sh
#
# Downloads a prebuilt `jama` binary from the repo's GitHub releases,
# installs it (default: ~/.local/bin, override with $JAMA_INSTALL_DIR),
# and symlinks the `jm` alias next to it. No network call happens after
# this script finishes — JAMA itself makes none, ever.

set -eu

REPO="Mimf-India/jama"
INSTALL_DIR="${JAMA_INSTALL_DIR:-$HOME/.local/bin}"

os() {
    case "$(uname -s)" in
        Linux) echo "unknown-linux-musl" ;;
        Darwin) echo "apple-darwin" ;;
        *) echo "unsupported platform: $(uname -s)" >&2; exit 1 ;;
    esac
}

arch() {
    case "$(uname -m)" in
        x86_64 | amd64) echo "x86_64" ;;
        arm64 | aarch64) echo "aarch64" ;;
        *) echo "unsupported architecture: $(uname -m)" >&2; exit 1 ;;
    esac
}

TARGET="$(arch)-$(os)"
ASSET="jama-${TARGET}.tar.gz"
URL="https://github.com/${REPO}/releases/latest/download/${ASSET}"

echo "Installing jama (${TARGET}) to ${INSTALL_DIR} ..."
mkdir -p "$INSTALL_DIR"

TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

curl -fsSL "$URL" -o "$TMP_DIR/$ASSET"
tar -xzf "$TMP_DIR/$ASSET" -C "$TMP_DIR"
install -m 755 "$TMP_DIR/jama" "$INSTALL_DIR/jama"
ln -sf "$INSTALL_DIR/jama" "$INSTALL_DIR/jm"

echo "Installed: $INSTALL_DIR/jama (alias: jm)"
case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *) echo "Note: $INSTALL_DIR is not on your \$PATH — add it, e.g. export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
esac
echo "Run 'jama --version' to confirm, then 'jama init ~/money' to get started."
