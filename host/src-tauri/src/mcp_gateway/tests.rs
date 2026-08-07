use super::origin_allowed;

#[test]
fn origin_validation_accepts_only_exact_loopback_authorities() {
    assert!(origin_allowed(None, &[]));
    assert!(origin_allowed(Some("http://localhost:8137"), &[]));
    assert!(origin_allowed(Some("https://127.0.0.1"), &[]));
    assert!(origin_allowed(Some("http://[::1]:8137"), &[]));
    assert!(!origin_allowed(Some("https://localhost.evil.example"), &[]));
    assert!(!origin_allowed(Some("http://localhost:bad"), &[]));
    assert!(!origin_allowed(Some("http://user@localhost"), &[]));
    assert!(!origin_allowed(Some("http://localhost/path"), &[]));
}
