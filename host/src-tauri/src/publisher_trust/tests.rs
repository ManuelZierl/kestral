use super::*;
use app_host_kernel::ids::AppId;
use base64::engine::general_purpose::STANDARD;
use ed25519_dalek::SigningKey;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

fn scratch_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("publisher-trust-{label}-{}", uuid::Uuid::new_v4()))
}

fn key_material(seed: u8) -> (String, String) {
    let signing_key = SigningKey::from_bytes(&[seed; 32]);
    let public_key = signing_key.verifying_key();
    let public_key_b64 = STANDARD.encode(public_key.as_bytes());
    let key_id = format!("ed25519:{:x}", Sha256::digest(public_key.as_bytes()));
    (key_id, public_key_b64)
}

#[test]
fn trust_store_persists_trust_and_revocation() {
    let path = scratch_path("persist");
    let (key_id, public_key) = key_material(1);
    let scope = TrustScope::AppId {
        app_id: AppId::new("com.example.publisher"),
    };

    {
        let mut store = PublisherTrustStore::new(path.clone()).unwrap();
        store
            .trust_key(&key_id, &public_key, scope.clone())
            .unwrap();
        assert_eq!(store.list().len(), 1);
        store.revoke_key(&key_id, &scope).unwrap();
        assert_eq!(store.list()[0].status, TrustStatus::Revoked);
    }

    let store = PublisherTrustStore::new(path).unwrap();
    assert_eq!(store.list().len(), 1);
    assert_eq!(store.list()[0].key_id, key_id);
    assert_eq!(store.list()[0].status, TrustStatus::Revoked);
}

#[test]
fn trust_store_rejects_invalid_key_material() {
    let mut store = PublisherTrustStore::in_memory();
    let scope = TrustScope::NamespacePrefix {
        namespace_prefix: "com.example".into(),
    };

    let error = store
        .trust_key("ed25519:deadbeef", "not-base64", scope)
        .unwrap_err();
    assert!(error.contains("invalid base64 public key"), "{error}");
}
