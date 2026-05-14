#!/bin/sh
# ViewScript CLI Installer
# Usage: curl -fsSL https://viewscript.pages.dev/install.sh | sh
#
# Environment variables:
#   VSC_INSTALL_DIR  - Installation directory (default: ~/.local/bin)
#   VSC_VERSION      - Specific version to install (default: latest)

set -e

REPO="viewscript/viewscript"
INSTALL_DIR="${VSC_INSTALL_DIR:-$HOME/.local/bin}"
BINARY_NAME="vsc"

# Colors (disabled if not a terminal)
if [ -t 1 ]; then
    RED='\033[0;31m'
    GREEN='\033[0;32m'
    YELLOW='\033[0;33m'
    BLUE='\033[0;34m'
    NC='\033[0m'
else
    RED=''
    GREEN=''
    YELLOW=''
    BLUE=''
    NC=''
fi

info() {
    printf "${BLUE}info${NC}: %s\n" "$1"
}

success() {
    printf "${GREEN}success${NC}: %s\n" "$1"
}

warn() {
    printf "${YELLOW}warn${NC}: %s\n" "$1"
}

error() {
    printf "${RED}error${NC}: %s\n" "$1" >&2
    exit 1
}

# Detect OS
detect_os() {
    case "$(uname -s)" in
        Linux*)  echo "linux" ;;
        Darwin*) echo "macos" ;;
        MINGW*|MSYS*|CYGWIN*) echo "windows" ;;
        *)       error "Unsupported operating system: $(uname -s)" ;;
    esac
}

# Detect architecture
detect_arch() {
    case "$(uname -m)" in
        x86_64|amd64)  echo "x86_64" ;;
        arm64|aarch64) echo "aarch64" ;;
        *)             error "Unsupported architecture: $(uname -m)" ;;
    esac
}

# Get latest version from Cloudflare Pages
get_latest_version() {
    curl -fsSL "https://viewscript.pages.dev/releases/version/latest" 2>/dev/null || \
    error "Failed to fetch latest version"
}

# Get download URL for the binary
get_download_url() {
    local os="$1"
    local arch="$2"
    local version="$3"

    local artifact=""
    case "${os}-${arch}" in
        linux-x86_64)   artifact="vsc-linux-x86_64" ;;
        macos-x86_64)   artifact="vsc-macos-x86_64" ;;
        macos-aarch64)  artifact="vsc-macos-aarch64" ;;
        windows-x86_64) artifact="vsc-windows-x86_64.exe" ;;
        *)              error "No binary available for ${os}-${arch}" ;;
    esac

    # GitHub Releases URL
    echo "https://github.com/${REPO}/releases/download/v${version}/${artifact}"
}

# Download with progress
download() {
    local url="$1"
    local output="$2"

    if command -v curl >/dev/null 2>&1; then
        curl -fsSL --progress-bar -o "$output" "$url"
    elif command -v wget >/dev/null 2>&1; then
        wget -q --show-progress -O "$output" "$url"
    else
        error "Neither curl nor wget found. Please install one of them."
    fi
}

# Main installation
main() {
    info "ViewScript CLI Installer"
    echo ""

    # Detect platform
    OS=$(detect_os)
    ARCH=$(detect_arch)
    info "Detected platform: ${OS}-${ARCH}"

    # Get version
    if [ -n "$VSC_VERSION" ]; then
        VERSION="$VSC_VERSION"
        info "Installing specified version: ${VERSION}"
    else
        VERSION=$(get_latest_version)
        info "Latest version: ${VERSION}"
    fi

    # Create install directory
    mkdir -p "$INSTALL_DIR"

    # Download URL
    DOWNLOAD_URL=$(get_download_url "$OS" "$ARCH" "$VERSION")
    info "Downloading from: ${DOWNLOAD_URL}"

    # Temporary file
    TMP_FILE=$(mktemp)
    trap "rm -f $TMP_FILE" EXIT

    # Download
    if ! download "$DOWNLOAD_URL" "$TMP_FILE"; then
        # Fallback: try to download WASI binary and extract from embedded
        warn "Pre-built binary not found. Trying WASI fallback..."

        WASI_URL="https://viewscript.pages.dev/releases/bin/vsc-core.wasm.zst"
        info "This platform may require wasmtime. Download WASI binary manually:"
        info "  curl -fsSL ${WASI_URL} -o vsc-core.wasm.zst"
        info "  zstd -d vsc-core.wasm.zst"
        info "  wasmtime run vsc-core.wasm -- --help"
        error "Pre-built native binary not available for ${OS}-${ARCH}"
    fi

    # Install
    INSTALL_PATH="${INSTALL_DIR}/${BINARY_NAME}"
    if [ "$OS" = "windows" ]; then
        INSTALL_PATH="${INSTALL_PATH}.exe"
    fi

    mv "$TMP_FILE" "$INSTALL_PATH"
    chmod +x "$INSTALL_PATH"

    success "Installed vsc to ${INSTALL_PATH}"
    echo ""

    # Check if in PATH
    if ! echo "$PATH" | tr ':' '\n' | grep -q "^${INSTALL_DIR}$"; then
        warn "${INSTALL_DIR} is not in your PATH"
        echo ""
        echo "Add the following to your shell profile (~/.bashrc, ~/.zshrc, etc.):"
        echo ""
        echo "  export PATH=\"\$PATH:${INSTALL_DIR}\""
        echo ""
    fi

    # Verify installation
    if command -v vsc >/dev/null 2>&1; then
        echo ""
        info "Verifying installation..."
        vsc --version 2>/dev/null || true
    else
        echo ""
        info "Run 'vsc --help' to get started (after adding to PATH)"
    fi

    echo ""
    success "Installation complete!"
}

main "$@"
