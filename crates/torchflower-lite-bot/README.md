# TorchFlower Lite Bot

`torchflower-lite-bot` is an extremely lightweight, low-resource Minecraft Bedrock AFK bot runtime. It is optimized to run hundreds of concurrent AFK bots on low-resource virtual private servers (VPS) with very low memory (<1 MB RSS per bot) and minimal CPU footprint.

---

## Key Features & Comparison

### How It Differs from the Full TorchFlower Engine

* **Shared Process & Runtime**: Running bots share a single system process and a single-threaded Tokio async event loop (rather than spawning multiple OS threads or processes per bot).
* **Minimalist Dependencies**: Bypasses heavy dashboard, web API, database migration, or telemetry systems by disabling `full-engine` features inside `torchflower-engine`.
* **No Cache Overhead**: Disables world chunk caching, entities history tracking, and packet logs by default.
* **Instant Script Hooks**: Actions (e.g. GUI click, respawn) run directly in the network loop, eliminating polling and scheduling overhead.

---

## Installation

### 1. Direct cargo installation (Linux/macOS)
You can install the lite bot binary directly from GitHub:
```bash
cargo install --git https://github.com/Osamu-GWAD/TorchFlower \
  --package torchflower-lite-bot \
  --branch main \
  --locked \
  --force
```

### 2. Linux Install Script (One-Liner)
To install on a Linux VPS without manually configuring Cargo, run:
```bash
curl -fsSL https://raw.githubusercontent.com/Osamu-GWAD/TorchFlower/main/scripts/install-lite-bot.sh | bash
```

### 3. Windows Installation (PowerShell)
Install Git, Rustup, and Visual Studio Build Tools via WinGet, then build:
```powershell
winget install --id Git.Git -e
winget install --id Rustlang.Rustup -e
winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--wait --quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"

cargo install --git https://github.com/Osamu-GWAD/TorchFlower --package torchflower-lite-bot --branch main --locked --force
```

If the command `torchflower-lite-bot` is not found after installation, append the Cargo bin path:
```powershell
$env:Path += ";$env:USERPROFILE\.cargo\bin"
```

---

## CLI Usage & Commands

### Initialize Config
Generate a default `bots.toml` configuration:
```bash
torchflower-lite-bot init
```

### Run Bots
Connect and run bots from the configuration file:
```bash
torchflower-lite-bot run --config bots.toml
```

### Benchmark
Perform concurrent connection benchmark to measure memory usage under load:
```bash
torchflower-lite-bot bench --bots 100 --duration 10m
```

---

## Tuning for Ultra-Low RAM (VPS)

Set the following environment variables when running in extremely low-resource environments:

```bash
# Reduce logging noise and allocations
export RUST_LOG=warn
export BEDROCK_TRACE_PACKETS=0
export BEDROCK_TRACE_CHUNKS=0

# Limit Tokio worker threads to 1 and reduce stack size per thread to 256KB
export TORCHFLOWER_WORKERS=1
export TORCHFLOWER_THREAD_STACK_BYTES=262144
```

---

## Publishing to crates.io Note

This package uses internal workspace dependencies with local path declarations (e.g. `path = "../torchflower-engine"`). To publish this crate to crates.io:
1. All local dependencies must be published to crates.io first.
2. The `path` fields in `Cargo.toml` must be removed or replaced with versioned declarations (e.g., `version = "x.y.z"`).
3. The `publish = false` directive in the workspace/crate manifests must be removed.


