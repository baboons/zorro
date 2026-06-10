//! Tests for the release version-check helpers.

use zorro_core::update::{is_newer, parse_latest_tag};

#[test]
fn parses_tag_name_and_strips_v() {
    let json = r#"{"url":"…","tag_name":"v0.3.1","name":"0.3.1"}"#;
    assert_eq!(parse_latest_tag(json).as_deref(), Some("0.3.1"));

    // Without a leading v, and with whitespace.
    let json2 = r#"{ "tag_name" : "1.2.0" }"#;
    assert_eq!(parse_latest_tag(json2).as_deref(), Some("1.2.0"));
}

#[test]
fn missing_tag_is_none() {
    assert_eq!(parse_latest_tag(r#"{"message":"Not Found"}"#), None);
}

#[test]
fn version_comparison() {
    assert!(is_newer("0.2.0", "0.1.0"));
    assert!(is_newer("0.1.1", "0.1.0"));
    assert!(is_newer("1.0.0", "0.9.9"));
    assert!(!is_newer("0.1.0", "0.1")); // 0.1.0 == 0.1.(0) → not newer
    assert!(!is_newer("0.1.0", "0.1.0"));
    assert!(!is_newer("0.1.0", "0.2.0"));
    assert!(!is_newer("0.1.0", "1.0.0"));
    // Prerelease suffix is ignored for the numeric compare.
    assert!(is_newer("0.2.0-rc1", "0.1.0"));
}
