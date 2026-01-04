#![no_main]

use libfuzzer_sys::fuzz_target;
use nebula_value::Value;

fuzz_target!(|data: &[u8]| {
    // Try to parse arbitrary bytes as JSON
    if let Ok(s) = std::str::from_utf8(data) {
        // Attempt to deserialize
        if let Ok(value) = serde_json::from_str::<Value>(s) {
            // If deserialization succeeds, serialize back
            if let Ok(json) = serde_json::to_string(&value) {
                // And try to deserialize again (roundtrip)
                let _ = serde_json::from_str::<Value>(&json);
            }

            // Also try pretty printing
            let _ = serde_json::to_string_pretty(&value);
        }
    }
});