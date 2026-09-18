//! JSON configuration for the numeric execution policy exposed to JavaScript.

use serde::Deserialize;

#[cfg(feature = "source")]
use velin::ExecutionPolicy;
#[cfg(all(feature = "runtime", not(feature = "source")))]
use velin_vm::ExecutionPolicy;

const MAX_POLICY_JSON_BYTES: usize = 64 * 1024;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyConfig {
    max_fuel: Option<u64>,
    max_immediate_fuel: Option<u64>,
    max_host_effects: Option<usize>,
    max_call_depth: Option<usize>,
    max_value_values: Option<usize>,
    max_value_text_bytes: Option<usize>,
    max_machine_values: Option<usize>,
    max_machine_text_bytes: Option<usize>,
    max_host_payload_values: Option<usize>,
    max_host_payload_text_bytes: Option<usize>,
    max_host_queue_events: Option<usize>,
    max_host_queue_values: Option<usize>,
    max_host_queue_text_bytes: Option<usize>,
    progress_interval: Option<u64>,
}

/// Parses a bounded JSON policy object, applying omitted fields from defaults.
pub fn parse(json: &str) -> Result<ExecutionPolicy, String> {
    if json.len() > MAX_POLICY_JSON_BYTES {
        return Err("execution policy JSON exceeds 64 KiB".to_owned());
    }
    let config: PolicyConfig = serde_json::from_str(json)
        .map_err(|error| format!("invalid execution policy JSON: {error}"))?;
    let mut policy = ExecutionPolicy::default();
    if let Some(value) = config.max_fuel {
        policy.max_fuel = value;
    }
    if let Some(value) = config.max_immediate_fuel {
        policy.max_immediate_fuel = value;
    }
    if let Some(value) = config.max_host_effects {
        policy.max_host_effects = value;
    }
    if let Some(value) = config.max_call_depth {
        policy.max_call_depth = value;
    }
    if let Some(value) = config.max_value_values {
        policy.max_value_values = value;
    }
    if let Some(value) = config.max_value_text_bytes {
        policy.max_value_text_bytes = value;
    }
    if let Some(value) = config.max_machine_values {
        policy.max_machine_values = value;
    }
    if let Some(value) = config.max_machine_text_bytes {
        policy.max_machine_text_bytes = value;
    }
    if let Some(value) = config.max_host_payload_values {
        policy.max_host_payload_values = value;
    }
    if let Some(value) = config.max_host_payload_text_bytes {
        policy.max_host_payload_text_bytes = value;
    }
    if let Some(value) = config.max_host_queue_events {
        policy.max_host_queue_events = value;
    }
    if let Some(value) = config.max_host_queue_values {
        policy.max_host_queue_values = value;
    }
    if let Some(value) = config.max_host_queue_text_bytes {
        policy.max_host_queue_text_bytes = value;
    }
    if let Some(value) = config.progress_interval {
        policy.progress_interval = value.max(1);
    }
    Ok(policy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn omitted_fields_use_defaults_and_limits_are_applied() {
        let policy = parse(r#"{"max_fuel":7,"max_host_effects":2}"#).unwrap();
        assert_eq!(policy.max_fuel, 7);
        assert_eq!(policy.max_host_effects, 2);
        assert_eq!(
            policy.max_immediate_fuel,
            ExecutionPolicy::default().max_immediate_fuel
        );
    }

    #[test]
    fn malformed_unknown_and_oversized_policies_are_rejected() {
        assert!(parse("{").is_err());
        assert!(parse(r#"{"unknown":1}"#).is_err());
        assert!(
            parse(&format!(
                r#"{{"max_fuel":"{}"}}"#,
                "x".repeat(MAX_POLICY_JSON_BYTES)
            ))
            .is_err()
        );
    }
}
