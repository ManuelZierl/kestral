use super::*;
use std::io::Cursor;

#[test]
fn sse_parser_yields_data_payloads_and_joins_multiline_data() {
    let stream = "event: message\ndata: {\"a\":\ndata: 1}\n\n: comment\n\ndata: second\n\n";
    let mut reader = Cursor::new(stream);
    assert_eq!(
        next_sse_data(&mut reader).unwrap(),
        Some("{\"a\":\n1}".to_string())
    );
    assert_eq!(
        next_sse_data(&mut reader).unwrap(),
        Some("second".to_string())
    );
    assert_eq!(next_sse_data(&mut reader).unwrap(), None);
}

#[test]
fn non_http_endpoints_are_rejected() {
    assert!(StreamableHttpTransport::new("ftp://mcp.example").is_err());
    assert!(StreamableHttpTransport::new("http://127.0.0.1:9/mcp").is_ok());
}

#[test]
fn authenticated_endpoints_fail_closed_on_unsafe_transport_or_headers() {
    assert!(StreamableHttpTransport::with_headers(
        "http://mcp.example/mcp",
        vec![("Authorization".into(), "Bearer secret".into())],
    )
    .is_err());
    assert!(StreamableHttpTransport::with_headers(
        "http://127.0.0.1:9/mcp",
        vec![("Authorization".into(), "Bearer secret".into())],
    )
    .is_ok());
    assert!(StreamableHttpTransport::with_headers(
        "https://user:secret@mcp.example/mcp",
        Vec::new(),
    )
    .is_err());
    assert!(
        StreamableHttpTransport::with_headers("https://mcp.example/mcp#fragment", Vec::new(),)
            .is_err()
    );
    assert!(StreamableHttpTransport::with_headers(
        "https://mcp.example/mcp?access_token=secret",
        Vec::new(),
    )
    .is_err());
    assert!(StreamableHttpTransport::with_headers(
        "https://mcp.example/mcp",
        vec![("Mcp-Session-Id".into(), "override".into())],
    )
    .is_err());
    let error = StreamableHttpTransport::with_headers(
        "https://mcp.example/mcp",
        vec![("Authorization".into(), "secret\r\nInjected: value".into())],
    )
    .err()
    .unwrap()
    .to_string();
    assert!(!error.contains("secret"));
}
