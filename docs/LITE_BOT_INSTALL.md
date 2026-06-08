# TorchFlower Lite Bot Install Guide

This guide explains how to install and run `torchflower-lite-bot` without copying the full repository manually.

## Linux install with Cargo

```bash
sudo apt update
sudo apt install -y build-essential curl git

curl https://sh.rustup.rs -sSf | sh -s -- -y
source "$HOME/.cargo/env"

cargo install --git https://github.com/Osamu-GWAD/TorchFlower \
  --package torchflower-lite-bot \
  --branch main \
  --locked \
  --force
```

Check install:

```bash
torchflower-lite-bot --help
```

## Linux install with script

```bash
curl -fsSL https://raw.githubusercontent.com/Osamu-GWAD/TorchFlower/main/scripts/install-lite-bot.sh | bash
```

## Windows install

Open PowerShell:

```powershell
winget install --id Git.Git -e
winget install --id Rustlang.Rustup -e
winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--wait --quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

Restart PowerShell, then run:

```powershell
cargo install --git https://github.com/Osamu-GWAD/TorchFlower --package torchflower-lite-bot --branch main --locked --force
```

If Windows cannot find the command:

```powershell
$env:Path += ";$env:USERPROFILE\.cargo\bin"
```

## Create config

```bash
torchflower-lite-bot init
```

This creates `bots.toml`.

## Run bots

```bash
torchflower-lite-bot run --config bots.toml
```

## Benchmark

```bash
torchflower-lite-bot bench --bots 100 --duration 10m
```

## Example config

```toml
[server]
host = "127.0.0.1"
port = 19132

[runtime]
log_level = "warn"
duration_secs = 0
reconnect = true

[[bots]]
username = "Bot_1"
mode = "kill-loop"

[[bots]]
username = "Bot_2"
mode = "kill-loop"
```

## Low-resource settings

```bash
RUST_LOG=warn
BEDROCK_TRACE_PACKETS=0
BEDROCK_TRACE_CHUNKS=0
TORCHFLOWER_WORKERS=1
TORCHFLOWER_THREAD_STACK_BYTES=262144
```

## Crates.io note

The package can be installed from GitHub with `cargo install --git`.

For crates.io publishing, all internal TorchFlower path dependencies must be published or converted to versioned dependencies.
