# TorchFlower Lite Bot

`torchflower-lite-bot` is the low-resource bot runner for TorchFlower.

It is designed for running many lightweight Minecraft Bedrock bot sessions in one process.

## Install from GitHub

```bash
cargo install --git https://github.com/Osamu-GWAD/TorchFlower \
  --package torchflower-lite-bot \
  --branch main \
  --locked \
  --force
```

## Quick Linux install

```bash
curl -fsSL https://raw.githubusercontent.com/Osamu-GWAD/TorchFlower/main/scripts/install-lite-bot.sh | bash
```

## Windows install

Install Git, Rust, and Visual Studio Build Tools:

```powershell
winget install --id Git.Git -e
winget install --id Rustlang.Rustup -e
winget install --id Microsoft.VisualStudio.2022.BuildTools -e --override "--wait --quiet --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

Install the lite bot:

```powershell
cargo install --git https://github.com/Osamu-GWAD/TorchFlower --package torchflower-lite-bot --branch main --locked --force
```

If the command is not found:

```powershell
$env:Path += ";$env:USERPROFILE\.cargo\bin"
```

## Usage

Create a config:

```bash
torchflower-lite-bot init
```

Run bots:

```bash
torchflower-lite-bot run --config bots.toml
```

Run a benchmark:

```bash
torchflower-lite-bot bench --bots 100 --duration 10m
```

## Low-RAM environment variables

```bash
RUST_LOG=warn
BEDROCK_TRACE_PACKETS=0
BEDROCK_TRACE_CHUNKS=0
TORCHFLOWER_WORKERS=1
TORCHFLOWER_THREAD_STACK_BYTES=262144
```

## Notes

`cargo install --git` works before crates.io publishing.

For crates.io publishing, all local path dependencies must either be published in order or replaced with versioned dependencies.
