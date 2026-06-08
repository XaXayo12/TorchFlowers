# Installation & Setup Guide: TorchFlower Lite Bot

This guide explains how to install and configure `torchflower-lite-bot` on Windows and Linux VPS.

---

## 1. Quick Install

### Linux VPS One-Liner
Install Rust and `torchflower-lite-bot` automatically with:
```bash
curl -fsSL https://raw.githubusercontent.com/Osamu-GWAD/TorchFlower/main/scripts/install-lite-bot.sh | bash
```

### Windows PowerShell One-Liner
Run PowerShell as Administrator and execute:
```powershell
winget install --id Git.Git -e; winget install --id Rustlang.Rustup -e; winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--wait --quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
# Restart your shell or refresh env, then run:
cargo install --git https://github.com/Osamu-GWAD/TorchFlower --package torchflower-lite-bot --branch main --locked --force
```

---

## 2. Configuration (`bots.toml`)

Initialize a default config:
```bash
torchflower-lite-bot init
```

Edit the generated `bots.toml` to customize your bots:
```toml
[server]
host = "127.0.0.1"
port = 19132

[runtime]
log_level = "warn"
duration_secs = 0     # 0 means run indefinitely
reconnect = true

[[bots]]
username = "AFKBot_1"
mode = "kill-loop"

[[bots]]
username = "AFKBot_2"
mode = "kill-loop"
```

---

## 3. Running

To start the bots:
```bash
torchflower-lite-bot run --config bots.toml
```

To run a benchmark (e.g. 100 bots for 5 minutes):
```bash
torchflower-lite-bot bench --bots 100 --duration 5m
```

---

## 4. Environment Optimization for VPS

To achieve maximum efficiency (<1 MB RSS per bot), set these environment variables before running:
```bash
export RUST_LOG=warn
export BEDROCK_TRACE_PACKETS=0
export BEDROCK_TRACE_CHUNKS=0
export TORCHFLOWER_WORKERS=1
export TORCHFLOWER_THREAD_STACK_BYTES=262144
```
