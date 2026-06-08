# TorchFlower Lite Bot

`torchflower-lite-bot` is the low-resource bot runner for TorchFlower.

It is designed for running many lightweight Minecraft Bedrock bot sessions in one process.

## Install from GitHub

```bash
cargo install --git https://github.com/Osamu-GWAD/TorchFlower \
  torchflower-lite-bot \
  --branch main \
  --locked \
  --force
```

From a local checkout:

```bash
cargo install --path crates/torchflower-lite-bot --locked --force
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
cargo install --git https://github.com/Osamu-GWAD/TorchFlower torchflower-lite-bot --branch main --locked --force
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

## Protocol Version Override

You can override the Bedrock protocol version used by the bot runner. The priority order is:

1. `[server] protocol_version = XXX`
2. `TORCHFLOWER_BEDROCK_PROTOCOL_VERSION`
3. `BEDROCK_PROTOCOL_VERSION`
4. TorchFlower's default protocol constant

### Using environment variables

On Windows:
```powershell
$env:TORCHFLOWER_BEDROCK_PROTOCOL_VERSION = "XXX"
torchflower-lite-bot run --config .\bots.toml
```

On Linux:
```bash
export TORCHFLOWER_BEDROCK_PROTOCOL_VERSION="XXX"
torchflower-lite-bot run --config ./bots.toml
```

### Using configuration file

You can also set the protocol version directly in `bots.toml` under the `[server]` section:

```toml
[server]
host = "127.0.0.1"
port = 19132
protocol_version = XXX
```

For bounded probing, omit `protocol_version` and set a list:

```toml
[server]
host = "127.0.0.1"
port = 19132
protocol_versions = [893, 898, 899]
```

The runner tries each configured version once for a NetworkSettings failure and then stops the probe.

## NetworkSettings Diagnostics

An early disconnect before `NetworkSettingsPacket` means the server rejected the client before login. Common causes are an unsupported protocol version, invalid RequestNetworkSettings encoding/framing, or a server-side rejection before authentication starts.

Enable debug logs when investigating:

```bash
RUST_LOG=debug torchflower-lite-bot run --config ./bots.toml
```

The NetworkSettings logs include the requested protocol version, codec protocol version, payload length, and debug-only hex bytes.

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
