#!/bin/bash

set -euo pipefail

# Parse arguments
BUILD_TYPE="debug"
if [[ "$#" -gt 0 && "$1" == "--release" ]]; then
    BUILD_TYPE="release"
fi

mkdir -p ./target/gitwrap/bin

echo "Creating symlinks from in gitwrap folder to target/$BUILD_TYPE"
ln -sf $(pwd)/target/$BUILD_TYPE/git-ai $(pwd)/target/gitwrap/bin/git
ln -sf $(pwd)/target/$BUILD_TYPE/git-ai $(pwd)/target/gitwrap/bin/git-ai

echo "In your shell profile,"
echo "1. Remove any artifacts from running 'install.sh'"
echo "2. Remove any aliases to git"
echo "3. Add the following to your shell profile:"
echo "# git-ai local dev"
echo "export PATH=\"$(pwd)/target/gitwrap/bin:\$PATH\""
