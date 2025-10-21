#!/usr/bin/env bash

# This script is used to run the GitHub integration tests.
# These tests create actual GitHub repositories and PRs, so are not included in the default test suite.

set -e

echo "🔍 Checking GitHub CLI availability..."
if ! command -v gh &> /dev/null; then
    echo "❌ GitHub CLI (gh) is not installed"
    echo "   Install from: https://cli.github.com/"
    exit 1
fi

if ! gh auth status &> /dev/null; then
    echo "❌ GitHub CLI is not authenticated"
    echo "   Run: gh auth login"
    exit 1
fi

echo "✅ GitHub CLI is available and authenticated"
echo ""
echo "🚀 Running GitHub integration tests..."
echo ""

cargo test --test github_integration -- --ignored --nocapture "$@"
