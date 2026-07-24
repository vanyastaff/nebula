use nebula_storage_port::SecretBytes;

#[test]
fn secret_bytes_debug_shape_is_independent_of_content_and_length() {
    let variants = [
        SecretBytes::new(Vec::new()),
        SecretBytes::new(vec![0x41]),
        SecretBytes::new(vec![0x5a; 4_096]),
    ];

    let rendered = variants.map(|secret| format!("{secret:?}"));

    assert_eq!(rendered[0], rendered[1]);
    assert_eq!(rendered[1], rendered[2]);
    assert!(!rendered[0].contains("bytes"));
}
