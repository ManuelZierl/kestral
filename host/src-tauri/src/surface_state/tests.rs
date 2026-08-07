use std::fs;

use app_host_kernel::ids::{AppId, SurfaceName};
use serde_json::json;

use super::{SurfaceStateStore, STORE_FILE};

fn object(value: serde_json::Value) -> app_host_kernel::JsonObject {
    value.as_object().unwrap().clone()
}

#[test]
fn state_is_scoped_by_app_surface_and_key() {
    let root = std::env::temp_dir().join(format!("kestral-surface-state-{}", uuid::Uuid::new_v4()));
    let store = SurfaceStateStore::new(root.clone());
    let app = AppId::new("org.example.one");
    let other_app = AppId::new("org.example.two");
    let surface = SurfaceName::new("panel");
    let other_surface = SurfaceName::new("other");

    let saved = store
        .put(
            &app,
            &surface,
            "message-1",
            0,
            Some(object(json!({"read": true}))),
        )
        .unwrap();
    assert_eq!(saved.revision, 1);
    assert_eq!(store.get(&app, &surface, "message-1").unwrap(), saved);
    assert_eq!(
        store.get(&app, &other_surface, "message-1").unwrap().value,
        None
    );
    assert_eq!(
        store.get(&other_app, &surface, "message-1").unwrap().value,
        None
    );

    let raw = fs::read_to_string(root.join(app.as_str()).join(STORE_FILE)).unwrap();
    assert!(raw.starts_with("{\n  \"version\": 2,\n  \"generation\": 1,\n"));

    fs::remove_dir_all(root).unwrap();
}

#[test]
fn compare_and_swap_rejects_stale_writes_and_keeps_tombstones() {
    let root = std::env::temp_dir().join(format!("kestral-surface-state-{}", uuid::Uuid::new_v4()));
    let store = SurfaceStateStore::new(root.clone());
    let app = AppId::new("org.example.one");
    let surface = SurfaceName::new("panel");

    store
        .put(
            &app,
            &surface,
            "message-1",
            0,
            Some(object(json!({"value": 1}))),
        )
        .unwrap();
    let error = store
        .put(
            &app,
            &surface,
            "message-1",
            0,
            Some(object(json!({"value": 2}))),
        )
        .unwrap_err();
    assert!(error.contains("expected revision 0, found 1"));
    let path = root.join(app.as_str()).join(STORE_FILE);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&fs::read_to_string(&path).unwrap()).unwrap()
            ["generation"],
        1
    );
    let deleted = store.put(&app, &surface, "message-1", 1, None).unwrap();
    assert_eq!(deleted.revision, 2);
    assert_eq!(deleted.value, None);
    assert_eq!(store.get(&app, &surface, "message-1").unwrap().revision, 2);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&fs::read_to_string(&path).unwrap()).unwrap()
            ["generation"],
        2
    );
    assert!(path.exists());

    fs::remove_dir_all(root).unwrap();
}
