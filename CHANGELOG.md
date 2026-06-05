# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0-alpha] - Unreleased

### Added

- Microsoft device-code authentication with Xbox Live, standard XSTS, PlayFab XSTS, PlayFab login, Minecraft entitlement session initialization, legacy Bedrock authentication, and Bedrock JWT chain generation.
- SQLite-backed account, token, entitlement, server, bot, and diagnostic persistence.
- Authenticated Axum REST API with exact-origin CORS, loopback-only unauthenticated development mode, and redacted diagnostics defaults.
- Bedrock client session support for RakNet handshake, ACK/NACK handling, fragmentation, reassembly, NetworkSettings, ZLib compression, login, encryption, resource pack acknowledgement, client cache status, StartGame processing, spawn observation, keepalive, chat, movement, inventory observation, and disconnect handling.
- DonutSMP-compatible NetworkStackLatency response encoding.
- Real-server validation for login, spawn, remained-connected, keepalive, chat, movement, inventory transactions, block-breaking evidence, and guarded block placing.
- Public `torchflower_engine::core` API types and examples for login, connect, chat, movement, block pickup, block placing, multi-bot supervision, and authenticated local API usage.

### Security

- API key authentication is required for `/api/*` routes by default.
- Token encryption uses strong key validation and redacted auth diagnostics.
- Server validation by raw host is restricted by `TORCHFLOWER_ALLOWED_SERVER_HOSTS`.
