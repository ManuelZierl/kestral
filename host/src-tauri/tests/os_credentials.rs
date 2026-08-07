use std::fs;

use app_host_kernel::ids::{AppId, SecretName, SecretRef};
use host_lib::config::{OsProtectedSecretStore, SecretStorage};
use uuid::Uuid;

/// Run explicitly on each release platform. Linux requires an unlocked Secret
/// Service session; CI/headless environments must not substitute plaintext.
#[test]
#[ignore = "requires the platform OS credential service"]
fn native_credential_round_trip_keeps_json_status_only() {
    let directory = std::env::temp_dir().join(format!("os-credential-test-{}", Uuid::new_v4()));
    let path = directory.join("host-secrets.json");
    let mut store = OsProtectedSecretStore::new(path.clone()).unwrap();
    let ref_ = SecretRef {
        owner: AppId::new("credential-integration-test"),
        name: SecretName::new(Uuid::new_v4().to_string()),
    };
    let value = format!("integration-secret-{}", Uuid::new_v4());

    store.write(&ref_, value.clone()).unwrap();
    assert_eq!(store.read(&ref_).unwrap().as_deref(), Some(value.as_str()));
    let json = fs::read_to_string(path).unwrap();
    assert!(!json.contains(&value));
    store.clear(&ref_).unwrap();
    assert!(!store.check(&ref_).unwrap());
}
