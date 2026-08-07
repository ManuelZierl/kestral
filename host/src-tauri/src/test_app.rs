//! Generic capability-provider fixture for host tests.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use app_host_kernel::ids::{AppId, ArtifactTypeName, CapabilityName};
use app_host_kernel::invocation::{CapabilityHandler, CapabilityOutcome, HandlerFailure};
use app_host_kernel::manifest::{seal, AppManifest, ArtifactTypeDeclaration, SealedManifest};
use app_host_kernel::primitives::artifact::ArtifactDraft;
use app_host_kernel::primitives::capability::{
    CapabilityDeclaration, CapabilityEffect, CapabilityRef,
};
use app_host_kernel::JsonObject;
use serde_json::{json, Value};

pub const TEST_APP_ID: &str = "com.example.workspace";
const ITEM_ARTIFACT: &str = "test-item";

#[derive(Default)]
pub struct TestAppStore {
    next_id: u64,
}

pub fn test_capability_ref(capability: &str) -> CapabilityRef {
    CapabilityRef {
        provider: AppId::new(TEST_APP_ID),
        capability: CapabilityName::new(capability),
    }
}

fn schema(value: Value) -> JsonObject {
    value
        .as_object()
        .cloned()
        .expect("test schema is an object")
}

fn declaration(
    name: &str,
    description: &str,
    effect: CapabilityEffect,
    input_schema: Value,
) -> CapabilityDeclaration {
    CapabilityDeclaration {
        name: CapabilityName::new(name),
        description: description.into(),
        input_schema: schema(input_schema),
        effect,
        output_schema: None,
    }
}

fn test_manifest() -> AppManifest {
    AppManifest {
        app_id: AppId::new(TEST_APP_ID),
        version: "1.0.0".into(),
        display_name: "Workspace fixture".into(),
        description: "Generic host test capability provider".into(),
        capabilities: vec![
            declaration(
                "list",
                "List fixture items.",
                CapabilityEffect::ReadOnly,
                json!({"type":"object","properties":{},"additionalProperties":false}),
            ),
            declaration(
                "search",
                "Search fixture items.",
                CapabilityEffect::ReadOnly,
                json!({
                    "type":"object",
                    "properties":{"query":{"type":"string"}},
                    "required":["query"],
                    "additionalProperties":false
                }),
            ),
            declaration(
                "create",
                "Create a fixture item.",
                CapabilityEffect::LocalWrite,
                json!({
                    "type":"object",
                    "properties":{
                        "title":{"type":"string","minLength":1},
                        "body":{"type":"string"}
                    },
                    "required":["title","body"],
                    "additionalProperties":false
                }),
            ),
            declaration(
                "write",
                "Update a fixture item.",
                CapabilityEffect::LocalWrite,
                json!({
                    "type":"object",
                    "properties":{
                        "target":{"type":"string","minLength":1},
                        "body":{"type":"string"}
                    },
                    "required":["target","body"],
                    "additionalProperties":false
                }),
            ),
            declaration(
                "delete",
                "Delete a fixture item.",
                CapabilityEffect::Destructive,
                json!({
                    "type":"object",
                    "properties":{"target":{"type":"string","minLength":1}},
                    "required":["target"],
                    "additionalProperties":false
                }),
            ),
        ],
        surfaces: vec![],
        agents: vec![],
        skills: vec![],
        assistant_profiles: vec![],
        automations: vec![],
        connectors: vec![],
        config_declarations: vec![],
        artifact_types: vec![ArtifactTypeDeclaration {
            name: ArtifactTypeName::new(ITEM_ARTIFACT),
            description: "Fixture item created by a test capability.".into(),
            json_schema: schema(json!({
                "type":"object",
                "properties":{
                    "item_id":{"type":"string"},
                    "title":{"type":"string"},
                    "body":{"type":"string"}
                },
                "required":["item_id","title","body"],
                "additionalProperties":false
            })),
        }],
        extension_points: vec![],
        extension_contributions: vec![],
        grant_requests: vec![],
        event_subscriptions: vec![],
    }
}

fn string_input(input: &JsonObject, name: &str) -> Result<String, HandlerFailure> {
    input
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| HandlerFailure(format!("{name} is required")))
}

fn test_handlers(store: Arc<Mutex<TestAppStore>>) -> BTreeMap<CapabilityName, CapabilityHandler> {
    let create_store = store;
    let create: CapabilityHandler = Box::new(move |input, _| {
        let title = string_input(input, "title")?;
        let body = string_input(input, "body")?;
        let item_id = {
            let mut store = create_store
                .lock()
                .map_err(|_| HandlerFailure("test app store lock poisoned".into()))?;
            store.next_id += 1;
            format!("item-{}", store.next_id)
        };
        let item = json!({"item_id": item_id, "title": title, "body": body});
        Ok(CapabilityOutcome {
            result: json!({"item": item.clone()}),
            artifacts: vec![ArtifactDraft {
                artifact_type: ArtifactTypeName::new(ITEM_ARTIFACT),
                title: format!("Fixture item: {title}"),
                content: item,
            }],
        })
    });
    let list: CapabilityHandler = Box::new(|_, _| {
        Ok(CapabilityOutcome {
            result: json!({"items": []}),
            artifacts: vec![],
        })
    });
    let search: CapabilityHandler = Box::new(|input, _| {
        let query = string_input(input, "query")?;
        Ok(CapabilityOutcome {
            result: json!({"query": query, "items": []}),
            artifacts: vec![],
        })
    });
    let write: CapabilityHandler = Box::new(|input, _| {
        Ok(CapabilityOutcome {
            result: json!({
                "item_id": string_input(input, "target")?,
                "body": string_input(input, "body")?
            }),
            artifacts: vec![],
        })
    });
    let delete: CapabilityHandler = Box::new(|input, _| {
        Ok(CapabilityOutcome {
            result: json!({"item_id": string_input(input, "target")?, "deleted": true}),
            artifacts: vec![],
        })
    });
    BTreeMap::from([
        (CapabilityName::new("list"), list),
        (CapabilityName::new("search"), search),
        (CapabilityName::new("create"), create),
        (CapabilityName::new("write"), write),
        (CapabilityName::new("delete"), delete),
    ])
}

pub fn test_app_install_parts(
    store: Arc<Mutex<TestAppStore>>,
) -> (SealedManifest, BTreeMap<CapabilityName, CapabilityHandler>) {
    (seal(test_manifest()), test_handlers(store))
}

#[cfg(test)]
pub(crate) fn install_test_app(
    kernel: &mut app_host_kernel::kernel::Kernel,
    store: Arc<Mutex<TestAppStore>>,
) -> app_host_kernel::KernelResult<()> {
    let (manifest, handlers) = test_app_install_parts(store);
    let prepared = kernel.prepare_install(manifest, handlers)?;
    kernel.commit_install(prepared.await_approval()).map(|_| ())
}
