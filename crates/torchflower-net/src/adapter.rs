//! TorchFlower-specific RakNet adapter policy.
//!
//! Keep this module thin. Anything generally useful for RakNet should be moved
//! upstream; this module only names the policy TorchFlower needs around timeouts
//! and frame sizing.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdapterPolicy {
    pub timeout_millis: u64,
    pub handshake_attempts: u8,
}

impl Default for AdapterPolicy {
    fn default() -> Self {
        Self {
            timeout_millis: 60_000,
            handshake_attempts: 5,
        }
    }
}

impl AdapterPolicy {
    pub fn from_env() -> Self {
        Self {
            timeout_millis: parse_timeout_millis(
                std::env::var("BEDROCK_RAKNET_TIMEOUT_MILLIS")
                    .ok()
                    .as_deref(),
            ),
            handshake_attempts: parse_handshake_attempts(
                std::env::var("BEDROCK_RAKNET_HANDSHAKE_ATTEMPTS")
                    .ok()
                    .as_deref(),
            ),
        }
    }
}

fn parse_timeout_millis(value: Option<&str>) -> u64 {
    value
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|millis| *millis >= 5_000)
        .unwrap_or(60_000)
}

fn parse_handshake_attempts(value: Option<&str>) -> u8 {
    value
        .and_then(|value| value.trim().parse::<u8>().ok())
        .filter(|attempts| *attempts > 0)
        .unwrap_or(5)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_policy_rejects_too_small_values() {
        assert_eq!(parse_timeout_millis(Some("100")), 60_000);
        assert_eq!(parse_timeout_millis(Some("5000")), 5_000);
    }

    #[test]
    fn handshake_attempts_default_on_zero_or_invalid() {
        assert_eq!(parse_handshake_attempts(Some("0")), 5);
        assert_eq!(parse_handshake_attempts(Some("3")), 3);
    }
}
