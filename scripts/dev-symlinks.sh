#!/bin/bash

set -x

mkdir -p ./target/gitwrap/bin

echo "Creating symlinks from in gitwrap folder to target/debug"
ln -sf $(pwd)/target/debug/git-ai $(pwd)/target/gitwrap/bin/git
ln -sf $(pwd)/target/debug/git-ai $(pwd)/target/gitwrap/bin/git-ai

echo "In your shell profile,"
echo "1. Remove any artifacts from running 'install.sh'"
echo "2. Remove any aliases to git"
echo "3. Add the following to your shell profile:"
echo "# git-ai local dev"
echo "export PATH=\"/Users/svarlamov/projects/git-ai/target/gitwrap/bin:\$PATH\""
