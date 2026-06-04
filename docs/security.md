# TorchFlower Security

TorchFlower controls authenticated Bedrock accounts and stores refresh tokens, so production deployments must use hardened defaults.

## API

- `/health` is public.
- `/api/*` requires `TORCHFLOWER_API_KEY`.
- API keys are accepted through `Authorization: Bearer <key>` or `X-TorchFlower-Api-Key`.
- Development unauthenticated mode requires `TORCHFLOWER_DEV_ALLOW_UNAUTH_API=true` and a loopback bind address.

## CORS

`TORCHFLOWER_CORS_ALLOWED_ORIGINS` is an exact comma-separated allowlist. Wildcard CORS is not used.

## Token Encryption

Use `TOKEN_ENCRYPTION_KEY_B64`, a base64-encoded 32-byte random key. Keep it stable for the database. Legacy `TOKEN_ENCRYPTION_SECRET` is accepted only when it is long enough and not a known placeholder.

## Diagnostics

Auth HTTP request and response bodies are not stored by default. `TORCHFLOWER_DANGEROUS_LOG_AUTH_BODIES=true` enables redacted body capture for local debugging only.

The redactor covers common secret fields including access tokens, refresh tokens, session tickets, authorization headers, identities, chains, signed tokens, secrets, passwords, and cookies.

## Automation Policy

Potentially disruptive gameplay actions such as attack, interact, break, place, use item, open inventory, and click slot require explicit `AutomationPolicy` opt-in and an allowed host. TorchFlower is for authorized accounts and servers only.
