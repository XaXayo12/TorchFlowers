## What this fixes

RakNet client stability issues observed during Bedrock login and gameplay validation, including split packet handling, ACK/NACK behavior, and resend pacing.

## Root cause

TorchFlower requires long-lived Bedrock sessions with large fragmented login and post-login packets. Generic RakNet behavior must preserve reliable ordering and recover from NACKs without stalling.

## How to reproduce before fix

Run a Bedrock client through login, resource pack negotiation, StartGame, movement, and gameplay probes against a local BDS/PocketMine server with large packets enabled.

## Behaviour after fix

The client should complete handshake, reassembly, reliable ACK/NACK recovery, and maintain ordered payload delivery without transport stalls.
