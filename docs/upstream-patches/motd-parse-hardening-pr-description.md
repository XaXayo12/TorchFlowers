## What this fixes

Malformed or non-standard MCPE MOTD responses can cause client-side parsing panics.

## Root cause

The parser accepts network-provided text fields and previously converted some numeric fields through panic-prone parsing.

## How to reproduce before fix

Ping a Bedrock server or proxy that returns a non-numeric MOTD field where the parser expects a number.

## Behaviour after fix

The parser returns a typed error and lets callers decide whether to ignore, retry, or report the malformed MOTD.
