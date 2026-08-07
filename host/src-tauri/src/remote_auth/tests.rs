use super::*;
use std::sync::Arc;

fn temp_root() -> PathBuf {
    std::env::temp_dir().join(format!("kestral-remote-auth-test-{}", Uuid::new_v4()))
}

fn auth(root: &Path) -> RemoteOwnerAuth {
    fs::create_dir_all(root).unwrap();
    RemoteOwnerAuth::open(
        root.join("remote-owner-auth-v1.json"),
        root.join("remote-owner-pairing-v1.json"),
        "https://kestral.example",
        None,
    )
    .unwrap()
}

#[test]
fn pairing_codes_are_hashed_single_use_and_start_server_side_state() {
    let root = temp_root();
    let pairing_path = root.join("remote-owner-pairing-v1.json");
    let code = create_pairing_code(&pairing_path).unwrap();
    let raw = fs::read_to_string(&pairing_path).unwrap();
    assert!(!raw.contains(&code));

    let mut auth = auth(&root);
    let registration = auth.start_registration(&code).unwrap();
    assert!(auth.registrations.contains_key(&registration.ceremony_id));
    assert!(!pairing_path.exists());
    assert_eq!(auth.start_registration(&code).unwrap_err().status, 401);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn expired_pairing_code_is_rejected_and_removed() {
    let root = temp_root();
    let pairing_path = root.join("remote-owner-pairing-v1.json");
    let now = Utc::now();
    let code = create_pairing_code_at(
        &pairing_path,
        now - Duration::minutes(PAIRING_LIFETIME_MINUTES + 1),
        standard_writer().as_ref(),
    )
    .unwrap();

    let mut auth = auth(&root);
    assert_eq!(auth.start_registration(&code).unwrap_err().status, 401);
    assert!(!pairing_path.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn owner_sessions_expire_on_idle_and_absolute_deadlines() {
    let root = temp_root();
    let mut auth = auth(&root);
    let now = Utc::now();
    let idle_token = auth.issue_session(now);
    assert!(auth.authenticate_session_at(
        &idle_token,
        now + Duration::minutes(SESSION_IDLE_MINUTES - 1)
    ));
    assert!(!auth.authenticate_session_at(
        &idle_token,
        now + Duration::minutes(SESSION_IDLE_MINUTES * 2)
    ));

    let absolute_token = auth.issue_session(now);
    for minute in (20..Duration::hours(SESSION_ABSOLUTE_HOURS).num_minutes()).step_by(20) {
        assert!(auth.authenticate_session_at(&absolute_token, now + Duration::minutes(minute)));
    }
    assert!(!auth.authenticate_session_at(
        &absolute_token,
        now + Duration::hours(SESSION_ABSOLUTE_HOURS)
    ));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn cookies_are_http_only_strict_and_secure_for_https() {
    let root = temp_root();
    let mut auth = auth(&root);
    let token = auth.issue_session(Utc::now());
    let cookie = auth.session_cookie(&token);
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Strict"));
    assert!(cookie.contains("Secure"));
    assert!(auth.authenticate_cookie(Some(&format!("other=x; {SESSION_COOKIE}={token}"))));
    auth.logout_cookie(Some(&format!("{SESSION_COOKIE}={token}")));
    assert!(!auth.authenticate_cookie(Some(&format!("{SESSION_COOKIE}={token}"))));
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn public_http_origin_and_changed_relying_party_fail_closed() {
    let root = temp_root();
    assert!(RemoteOwnerAuth::open(
        root.join("auth.json"),
        root.join("pairing.json"),
        "http://kestral.example",
        None,
    )
    .is_err());

    let document = OwnerAuthDocument {
        format_version: AUTH_FORMAT_VERSION,
        rp_id: "old.example".into(),
        origin: "https://old.example".into(),
        owner_user_id: Uuid::new_v4(),
        credentials: Vec::new(),
    };
    persist_json_document(
        &root.join("auth.json"),
        &document,
        "test auth",
        standard_writer().as_ref(),
    )
    .unwrap();
    let error = RemoteOwnerAuth::open_with_writer(
        root.join("auth.json"),
        root.join("pairing.json"),
        "https://new.example",
        None,
        Arc::new(crate::atomic_json::StandardAtomicFileWriter),
    )
    .err()
    .expect("changed origin must fail");
    assert!(error.contains("bound to origin") || error.contains("credential count"));
    fs::remove_dir_all(root).unwrap();
}
