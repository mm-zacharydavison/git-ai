#!/bin/bash

set -euo pipefail
IFS=$'\n\t'

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
NC='\033[0m' # No Color

# GitHub repository details
REPO="acunniffe/git-ai"

# Function to print error messages
error() {
    echo -e "${RED}Error: $1${NC}" >&2
    exit 1
}

# Function to print success messages
success() {
    echo -e "${GREEN}$1${NC}"
}

# Function to detect shell and generate alias command
detect_shell() {
    local shell_name=""
    local config_file=""
    
    # Check for zsh first (macOS default)
    if [ -f "$HOME/.zshrc" ]; then
        shell_name="zsh"
        config_file="$HOME/.zshrc"
    # Check for bash
    elif [ -f "$HOME/.bashrc" ] || [ -f "$HOME/.bash_profile" ]; then
        shell_name="bash"
        config_file="$HOME/.bashrc"
    else
        # Fallback - try to detect from environment
        if [ -n "$ZSH_VERSION" ]; then
            shell_name="zsh"
            config_file="$HOME/.zshrc"
        elif [ -n "$BASH_VERSION" ]; then
            shell_name="bash"
            config_file="$HOME/.bashrc"
        else
            shell_name="unknown"
            config_file=""
        fi
    fi
    
    echo "$shell_name|$config_file"
}

detect_std_git() {
    local git_path=""

    # Prefer the actual executable path, ignoring aliases and functions
    if git_path=$(type -P git 2>/dev/null); then
        :
    else
        git_path=$(command -v git 2>/dev/null || true)
    fi

    # Last resort
    if [ -z "$git_path" ]; then
        git_path=$(which git 2>/dev/null || true)
    fi

	# Ensure we never return a path for git that leads to ~/.git-ai (recursive)
	if [ -n "$git_path" ] && [[ "$git_path" == *"/.git-ai/"* ]]; then
		git_path=""
	fi

    echo "$git_path"
}

# Detect OS and architecture
OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

# Map architecture to binary name
case $ARCH in
    "x86_64")
        ARCH="x64"
        ;;
    "aarch64"|"arm64")
        ARCH="arm64"
        ;;
    *)
        error "Unsupported architecture: $ARCH"
        ;;
esac

# Map OS to binary name
case $OS in
    "darwin")
        OS="macos"
        ;;
    "linux")
        OS="linux"
        ;;
    *)
        error "Unsupported operating system: $OS"
        ;;
esac

# Determine binary name
BINARY_NAME="git-ai-${OS}-${ARCH}"

# Download URL
DOWNLOAD_URL="https://github.com/${REPO}/releases/latest/download/${BINARY_NAME}"

# Install into the user's bin directory ~/.git-ai/bin
INSTALL_DIR="$HOME/.git-ai/bin"

# Create directory if it doesn't exist
mkdir -p "$INSTALL_DIR"

# Download and install
echo "Downloading git-ai..."
TMP_FILE="${INSTALL_DIR}/git-ai.tmp.$$"
if ! curl --fail --location --silent --show-error -o "$TMP_FILE" "$DOWNLOAD_URL"; then
    rm -f "$TMP_FILE" 2>/dev/null || true
    error "Failed to download binary (HTTP error)"
fi

# Basic validation: ensure file is not empty
if [ ! -s "$TMP_FILE" ]; then
    rm -f "$TMP_FILE" 2>/dev/null || true
    error "Downloaded file is empty"
fi

mv -f "$TMP_FILE" "${INSTALL_DIR}/git-ai"

# Make executable
chmod +x "${INSTALL_DIR}/git-ai"
# Symlink git to git-ai
ln -sf "${INSTALL_DIR}/git-ai" "${INSTALL_DIR}/git"

# Remove quarantine attribute on macOS
if [ "$OS" = "macos" ]; then
    xattr -d com.apple.quarantine "${INSTALL_DIR}/git-ai" 2>/dev/null || true
fi

# Detect shell and get alias information
SHELL_INFO=$(detect_shell)
SHELL_NAME=$(echo "$SHELL_INFO" | cut -d'|' -f1)
CONFIG_FILE=$(echo "$SHELL_INFO" | cut -d'|' -f2)
STD_GIT_PATH=$(detect_std_git)
PATH_CMD="export PATH=\"$INSTALL_DIR:\$PATH\""

# Install hooks
echo "Setting up IDE/agent hooks..."
if ! ${INSTALL_DIR}/git-ai install-hooks; then
    echo "Warning: Failed to set up IDE/agent hooks; continuing without IDE/agent hooks." >&2
else
    success "Successfully set up IDE/agent hooks"
fi

success "Successfully installed git-ai into ${INSTALL_DIR}"
success "You can now run 'git-ai' from your terminal"

# Write JSON config at ~/.git-ai/config.json
CONFIG_DIR="$HOME/.git-ai"
CONFIG_JSON_PATH="$CONFIG_DIR/config.json"
mkdir -p "$CONFIG_DIR"

if [ -z "$STD_GIT_PATH" ]; then
    echo -e "${RED}Warning:${NC} Could not detect a standard git binary on PATH.\nYou can manually set it in: $CONFIG_JSON_PATH" >&2
fi

TMP_CFG="$CONFIG_JSON_PATH.tmp.$$"
cat >"$TMP_CFG" <<EOF
{
  "git_path": "${STD_GIT_PATH}",
  "ignore_prompts": false
}
EOF
mv -f "$TMP_CFG" "$CONFIG_JSON_PATH"

# Add to PATH automatically if not already there
if [[ ":$PATH:" != *"$INSTALL_DIR"* ]]; then
    if [ -n "$CONFIG_FILE" ]; then
        # Ensure config file exists
        touch "$CONFIG_FILE"
        # Append PATH update if not already present
        if ! grep -qsF "$INSTALL_DIR" "$CONFIG_FILE"; then
            echo "" >> "$CONFIG_FILE"
            echo "# Added by git-ai installer on $(date)" >> "$CONFIG_FILE"
            echo "$PATH_CMD" >> "$CONFIG_FILE"
        fi
        success "Updated ${CONFIG_FILE} to include ${INSTALL_DIR} in PATH"
        echo "Restart your shell or run: source \"$CONFIG_FILE\""
    else
        echo "Could not detect your shell config file."
        echo "Please add the following line(s) to your shell config and restart:"
        echo "$PATH_CMD"
    fi
fi
