# Upstreaming Protocol Work

TorchFlower should not permanently fork protocol behavior when the fix belongs in `bedrock-rs`.

Use this decision rule:

- Packet schema, protocol version mapping, serialization, deserialization, compression, and encryption primitives belong upstream.
- Bot policy, state tracking, movement strategy, inventory decisions, validation scenarios, and API security belong in TorchFlower.

When a local protocol adapter is required:

1. Add a minimal regression test with sanitized bytes.
2. Keep the adapter behind `bedrock::protocol_adapter`.
3. Document why the local adapter exists.
4. Prepare the smallest upstreamable patch separately.
