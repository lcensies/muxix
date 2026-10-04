#!/usr/bin/env bash
#
# muxix installation script
# Usage: curl -fsSL https://raw.githubusercontent.com/lcensies/muxix/main/scripts/install.sh | bash
#
# Environment variables:
#   MUXIX_VERSION      - Pin a specific version (e.g., v0.1.96)
#   MUXIX_INSTALL_DIR  - Override install directory (default: /usr/local/bin or ~/.local/bin)
#
# Examples:
#   MUXIX_VERSION=v0.1.96 bash install.sh
#   MUXIX_INSTALL_DIR=/opt/bin bash install.sh
#
# ⚠️ IMPORTANT: This script must be EXECUTED, never SOURCED
# ❌ WRONG: source install.sh (will exit your shell on errors)
# ✅ CORRECT: bash install.sh
# ✅ CORRECT: curl -fsSL ... | bash
#

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

log_info() {
	echo -e "${BLUE}==>${NC} $1"
}

log_success() {
	echo -e "${GREEN}==>${NC} $1"
}

log_warning() {
	echo -e "${YELLOW}==>${NC} $1"
}

log_error() {
	echo -e "${RED}Error:${NC} $1" >&2
}

# Detect OS and architecture
detect_platform() {
	local os arch

	case "$(uname -s)" in
	Darwin)
		os="darwin"
		;;
	Linux)
		os="linux"
		;;
	*)
		log_error "Unsupported operating system: $(uname -s)"
		echo ""
		echo "muxix supports macOS and Linux."
		echo "For other platforms, try building from source with Cargo:"
		echo "  cargo install muxix"
		echo ""
		exit 1
		;;
	esac

	case "$(uname -m)" in
	x86_64 | amd64)
		arch="amd64"
		;;
	aarch64 | arm64)
		arch="arm64"
		;;
	*)
		log_error "Unsupported architecture: $(uname -m)"
		echo ""
		echo "muxix prebuilt binaries are available for amd64 and arm64."
		echo "For other architectures, try building from source with Cargo:"
		echo "  cargo install muxix"
		echo ""
		exit 1
		;;
	esac

	echo "${os}-${arch}"
}

# Check required dependencies
check_dependencies() {
	local missing=()

	if ! command -v git &>/dev/null; then
		missing+=("git")
	fi

	if ! command -v tmux &>/dev/null; then
		missing+=("tmux")
	fi

	if [ ${#missing[@]} -gt 0 ]; then
		log_warning "Missing required dependencies: ${missing[*]}"
		echo ""
		echo "muxix requires git and tmux to function."
		echo "Please install them first:"
		echo "  - macOS: brew install git tmux"
		echo "  - Ubuntu/Debian: sudo apt install git tmux"
		echo "  - Other Linux: Use your distro's package manager"
		echo ""
		echo "Installation will continue, but muxix won't work until dependencies are installed."
		echo ""
	fi
}

# Download and install from GitHub releases
install_from_release() {
	log_info "Installing muxix from GitHub releases..."

	local platform=$1
	local tmp_dir
	tmp_dir=$(mktemp -d)
	trap 'rm -rf "$tmp_dir"' EXIT

	# Get latest release version (or use override)
	local version="${MUXIX_VERSION:-}"

	if [ -z "$version" ]; then
		log_info "Fetching latest release..."
		local latest_url="https://api.github.com/repos/lcensies/muxix/releases/latest"
		local release_json

		if command -v curl &>/dev/null; then
			release_json=$(curl -fsSL --retry 3 --retry-connrefused --connect-timeout 10 --max-time 30 "$latest_url")
		elif command -v wget &>/dev/null; then
			release_json=$(wget --tries=3 --timeout=30 -qO- "$latest_url")
		else
			log_error "Neither curl nor wget found. Please install one of them."
			exit 1
		fi

		version=$(echo "$release_json" | grep '"tag_name"' | sed -E 's/.*"tag_name": "([^"]+)".*/\1/')

		if [ -z "$version" ]; then
			log_error "Failed to fetch latest version"
			echo ""
			echo "This might be due to network issues or GitHub API rate limits."
			echo "You can specify a version manually:"
			echo "  MUXIX_VERSION=v0.1.96 bash install.sh"
			echo ""
			exit 1
		fi
	fi

	log_info "Installing version: $version"

	# Download URL
	local archive_name="muxix-${platform}.tar.gz"
	local download_url="https://github.com/lcensies/muxix/releases/download/${version}/${archive_name}"

	log_info "Downloading $archive_name..."

	cd "$tmp_dir"
	if command -v curl &>/dev/null; then
		if ! curl -fsSL --retry 3 --retry-connrefused --connect-timeout 10 --max-time 120 -o "$archive_name" "$download_url"; then
			log_error "Download failed"
			echo ""
			echo "The release may not have a prebuilt binary for your platform."
			echo "Try installing with Cargo instead:"
			echo "  cargo install muxix"
			echo ""
			cd - >/dev/null || cd "$HOME"
			exit 1
		fi
	elif command -v wget &>/dev/null; then
		if ! wget --tries=3 --timeout=120 -q -O "$archive_name" "$download_url"; then
			log_error "Download failed"
			echo ""
			echo "The release may not have a prebuilt binary for your platform."
			echo "Try installing with Cargo instead:"
			echo "  cargo install muxix"
			echo ""
			cd - >/dev/null || cd "$HOME"
			exit 1
		fi
	fi

	# Download and verify checksum
	log_info "Verifying checksum..."
	local checksum_file="${archive_name%.tar.gz}.sha256"
	local checksum_url="https://github.com/lcensies/muxix/releases/download/${version}/${checksum_file}"

	if command -v curl &>/dev/null; then
		if ! curl -fsSL --retry 3 --retry-connrefused --connect-timeout 10 --max-time 30 -o "$checksum_file" "$checksum_url"; then
			log_error "Failed to download checksum file"
			cd - >/dev/null || cd "$HOME"
			exit 1
		fi
	elif command -v wget &>/dev/null; then
		if ! wget --tries=3 --timeout=30 -q -O "$checksum_file" "$checksum_url"; then
			log_error "Failed to download checksum file"
			cd - >/dev/null || cd "$HOME"
			exit 1
		fi
	fi

	# Verify checksum using sha256sum (Linux) or shasum (macOS)
	if command -v sha256sum &>/dev/null; then
		if ! sha256sum -c "$checksum_file" &>/dev/null; then
			log_error "Checksum verification failed"
			echo ""
			echo "The downloaded file may be corrupted or tampered with."
			echo "Please try again or report this issue."
			echo ""
			cd - >/dev/null || cd "$HOME"
			exit 1
		fi
	elif command -v shasum &>/dev/null; then
		if ! shasum -a 256 -c "$checksum_file" &>/dev/null; then
			log_error "Checksum verification failed"
			echo ""
			echo "The downloaded file may be corrupted or tampered with."
			echo "Please try again or report this issue."
			echo ""
			cd - >/dev/null || cd "$HOME"
			exit 1
		fi
	else
		log_warning "Neither sha256sum nor shasum found, skipping checksum verification"
	fi

	log_success "Checksum verified"

	# Extract archive
	log_info "Extracting archive..."
	if ! tar -xzf "$archive_name"; then
		log_error "Failed to extract archive"
		exit 1
	fi

	# Determine install location (with override support)
	local install_dir="${MUXIX_INSTALL_DIR:-}"
	if [ -z "$install_dir" ]; then
		if [[ -w /usr/local/bin ]]; then
			install_dir="/usr/local/bin"
		else
			install_dir="$HOME/.local/bin"
			mkdir -p "$install_dir"
		fi
	fi

	# Check for existing installation
	if [ -f "$install_dir/muxix" ]; then
		local existing_version
		existing_version=$("$install_dir/muxix" --version 2>/dev/null | grep -oE 'v[0-9]+\.[0-9]+\.[0-9]+' || echo "unknown")
		log_info "Existing installation found: $existing_version"
		log_info "Upgrading to: $version"
	fi

	# Install binary atomically
	log_info "Installing to $install_dir..."
	local tmp_binary="$install_dir/muxix.tmp.$$"

	if [[ -w "$install_dir" ]]; then
		cp muxix "$tmp_binary"
		chmod +x "$tmp_binary"
		mv -f "$tmp_binary" "$install_dir/muxix"
	else
		if ! sudo cp muxix "$tmp_binary"; then
			log_error "Failed to install to $install_dir (sudo required)"
			exit 1
		fi
		sudo chmod +x "$tmp_binary"
		sudo mv -f "$tmp_binary" "$install_dir/muxix"
	fi

	# Remove macOS quarantine attribute if present
	if [[ "$(uname -s)" == "Darwin" ]] && command -v xattr &>/dev/null; then
		xattr -d com.apple.quarantine "$install_dir/muxix" 2>/dev/null || true
	fi

	log_success "muxix installed to $install_dir/muxix"

	# Check if install_dir is in PATH
	if [[ ":$PATH:" != *":$install_dir:"* ]]; then
		log_warning "$install_dir is not in your PATH"
		echo ""
		echo "Add this to your shell profile (~/.bashrc, ~/.zshrc, etc.):"
		echo "  export PATH=\"\$PATH:$install_dir\""
		echo ""
	fi

	cd - >/dev/null || cd "$HOME"

	# Return the install directory via a global variable
	INSTALL_DIR="$install_dir"
}

# Verify installation
verify_installation() {
	local install_dir="$1"

	# Verify the binary exists and is executable
	if [ ! -x "$install_dir/muxix" ]; then
		log_error "muxix binary not found or not executable at $install_dir/muxix"
		exit 1
	fi

	# Test the binary works
	if ! "$install_dir/muxix" --version &>/dev/null; then
		log_error "muxix binary exists but failed to run"
		exit 1
	fi

	log_success "muxix is installed and ready!"
	echo ""
	"$install_dir/muxix" --version
	echo ""

	# Check if install_dir is in PATH
	if ! command -v muxix &>/dev/null; then
		log_warning "$install_dir is not in your PATH"
		echo ""
		echo "Add this to your shell profile (~/.bashrc, ~/.zshrc, etc.):"
		echo "  export PATH=\"\$PATH:$install_dir\""
		echo ""
	fi

	echo "Get started:"
	echo "  cd your-project"
	echo "  muxix init         # (optional) create .muxix.yaml config"
	echo "  muxix add feature  # create a worktree and tmux window"
	echo ""
	echo "Recommended:"
	echo "  alias mx='muxix'   # add to your shell profile for faster typing"
	echo ""
	echo "Documentation: https://github.com/lcensies/muxix/blob/main/docs/index.md"
	echo ""
}

# Main installation flow
main() {
	echo ""
	echo "🚀 muxix installer"
	echo ""

	log_info "Detecting platform..."
	local platform
	platform=$(detect_platform)
	log_info "Platform: $platform"

	# Check dependencies (warn but don't fail)
	check_dependencies

	# Download and install
	install_from_release "$platform"

	# Verify
	verify_installation "$INSTALL_DIR"
}

main "$@"
