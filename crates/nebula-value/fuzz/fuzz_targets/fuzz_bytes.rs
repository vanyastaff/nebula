#![no_main]

use libfuzzer_sys::fuzz_target;
use nebula_value::Bytes;

fuzz_target!(|data: &[u8]| {
    // Create Bytes from arbitrary data
    let bytes = Bytes::new(data.to_vec());

    // Test various operations
    let _ = bytes.len();
    let _ = bytes.is_empty();
    let _ = bytes.as_slice();

    // Test base64 encoding
    let encoded = bytes.to_base64();

    // Try to decode back
    if let Ok(decoded) = Bytes::from_base64(&encoded) {
        // Should be identical
        assert_eq!(decoded.as_slice(), bytes.as_slice());
    }

    // Test slicing
    if data.len() > 1 {
        let _ = bytes.slice(0, data.len() / 2);
        let _ = bytes.slice(0, data.len());
    }

    // Test clone
    let cloned = bytes.clone();
    assert_eq!(cloned.as_slice(), bytes.as_slice());
});