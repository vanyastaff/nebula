#![no_main]

use libfuzzer_sys::fuzz_target;
use nebula_value::Text;

fuzz_target!(|data: &[u8]| {
    // Try to create Text from potentially invalid UTF-8
    if let Ok(s) = std::str::from_utf8(data) {
        let text = Text::from_str(s);

        // Test various operations
        let _ = text.len();
        let _ = text.is_empty();
        let _ = text.as_str();

        // Test concat
        let text2 = Text::from_str("test");
        let _ = text.concat(&text2);

        // Test clone
        let _ = text.clone();

        // Test substring if valid
        if !s.is_empty() && s.len() > 1 {
            let _ = text.substring(0, 1);
            let _ = text.substring(0, s.len());
        }
    }
});