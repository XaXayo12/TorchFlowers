#!/usr/bin/env bash
# Quick installer script for TorchFlower Lite Bot
set -euo pipefail

REPO_URL="https://github.com/Osamu-GWAD/TorchFlower"
PACKAGE="torchflower-lite-bot"
BRANCH="${TORCHFLOWER_BRANCH:-main}"

if ! command -v cargo >/dev/null 2>&1; then
  echo "Rust/Cargo not found. Installing rustup..."
  curl https://sh.rustup.rs -sSf | sh -s -- -y
  # shellcheck disable=SC1090
  source "$HOME/.cargo/env"
fi

echo "Installing $PACKAGE from $REPO_URL branch $BRANCH..."
cargo install \
  --git "$REPO_URL" \
  --package "$PACKAGE" \
  --branch "$BRANCH" \
  --locked \
  --force

echo "Installed:"
command -v torchflower-lite-bot
torchflower-lite-bot --help || true
