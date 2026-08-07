use super::*;

fn workspace(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("kestral-app-data-{label}-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    root
}

fn versioned(format_version: u32) -> AppData {
    AppData::Versioned {
        format_version,
        migration: AppDataMigration {
            protocol_version: 1,
            command: "node".into(),
            entry: "backend/migrate.mjs".into(),
            args: vec!["backend/migrate.mjs".into()],
            transitions: Vec::new(),
        },
    }
}

#[test]
fn initializes_and_reopens_versioned_data() {
    let root = workspace("initialize");
    let first = active_dir(&root, "com.example.app", "package-1", &versioned(1), "now").unwrap();
    fs::write(first.join("record.json"), b"one").unwrap();

    let reopened = active_dir(
        &root,
        "com.example.app",
        "package-1",
        &versioned(1),
        "later",
    )
    .unwrap();

    assert_eq!(first, reopened);
    assert_eq!(fs::read(reopened.join("record.json")).unwrap(), b"one");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn refuses_unversioned_existing_bytes() {
    let root = workspace("legacy");
    let app = root.join(".data/com.example.app");
    fs::create_dir_all(&app).unwrap();
    fs::write(app.join("record.json"), b"one").unwrap();

    let error =
        active_dir(&root, "com.example.app", "package-1", &versioned(1), "now").unwrap_err();

    assert!(error.contains("cannot be adopted automatically"));
    assert_eq!(fs::read(app.join("record.json")).unwrap(), b"one");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn stages_commits_and_rolls_back_without_touching_source() {
    let root = workspace("transition");
    let active = active_dir(&root, "com.example.app", "package-1", &versioned(1), "one").unwrap();
    fs::write(active.join("record.json"), b"source").unwrap();
    let source = current_revision(&root, "com.example.app").unwrap().unwrap();
    let candidate = AppDataRevision {
        revision_id: Uuid::new_v4().to_string(),
        format_version: 2,
        package_revision_id: "package-2".into(),
        created_at: "two".into(),
    };
    let (candidate_dir, _) = stage_candidate(
        &root,
        "com.example.app",
        &candidate,
        Some(&source.revision_id),
    )
    .unwrap();
    fs::write(candidate_dir.join("record.json"), b"candidate").unwrap();
    commit_candidate(
        &root,
        "com.example.app",
        Some(&source.revision_id),
        candidate.clone(),
    )
    .unwrap();

    rollback_transition(
        &root,
        "com.example.app",
        Some(&source.revision_id),
        &candidate.revision_id,
    )
    .unwrap();

    assert_eq!(
        fs::read(
            active_dir(&root, "com.example.app", "package-1", &versioned(1), "one")
                .unwrap()
                .join("record.json")
        )
        .unwrap(),
        b"source"
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn keeps_the_configured_number_of_backups() {
    let root = workspace("retention");
    active_dir(&root, "com.example.app", "package-1", &versioned(1), "1").unwrap();
    for (format, created_at) in [(2, "2"), (3, "3"), (4, "4")] {
        let source = current_revision(&root, "com.example.app").unwrap().unwrap();
        let candidate = AppDataRevision {
            revision_id: Uuid::new_v4().to_string(),
            format_version: format,
            package_revision_id: format!("package-{format}"),
            created_at: created_at.into(),
        };
        let _ = stage_candidate(
            &root,
            "com.example.app",
            &candidate,
            Some(&source.revision_id),
        )
        .unwrap();
        commit_candidate(
            &root,
            "com.example.app",
            Some(&source.revision_id),
            candidate,
        )
        .unwrap();
    }

    prune_backups(&root, "com.example.app", 1).unwrap();

    let state = load_state(&app_root(&root, "com.example.app")).unwrap();
    assert_eq!(state.revisions.len(), 2);
    assert_eq!(active_revision(&state).unwrap().format_version, 4);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn dedicated_migration_command_updates_only_the_candidate() {
    let node = std::process::Command::new("node")
        .arg("--version")
        .output()
        .expect("app-data migration test requires node on PATH");
    assert!(node.status.success());
    let root = workspace("command");
    let payload = root.join("payload");
    let candidate = root.join("candidate");
    fs::create_dir_all(payload.join("backend")).unwrap();
    fs::create_dir(&candidate).unwrap();
    fs::write(candidate.join("record.json"), br#"{"version":1}"#).unwrap();
    fs::write(
        payload.join("backend/migrate.mjs"),
        r#"import { createInterface } from "node:readline";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
const lines = createInterface({ input: process.stdin });
lines.on("line", (line) => {
  const request = JSON.parse(line);
  const path = join(process.env.APP_HOST_DATA_DIR, "record.json");
  const record = JSON.parse(readFileSync(path, "utf8"));
  record.version = request.params.to_format_version;
  writeFileSync(path, JSON.stringify(record));
  process.stdout.write(`${JSON.stringify({jsonrpc:"2.0",id:request.id,result:{protocol_version:1,format_version:record.version}})}\n`);
});
"#,
    )
    .unwrap();
    let migration = AppDataMigration {
        protocol_version: 1,
        command: "node".into(),
        entry: "backend/migrate.mjs".into(),
        args: vec!["backend/migrate.mjs".into()],
        transitions: vec![crate::package::AppDataTransition {
            from: 1,
            to: 2,
            destructive: false,
        }],
    };

    run_migration_command(
        &payload,
        &candidate,
        "com.example.app",
        &Backend::McpStdio {
            authority_mode: BackendAuthorityMode::Unsandboxed,
            command: "node".into(),
            args: Vec::new(),
        },
        &migration,
        1,
        2,
    )
    .unwrap();

    let record: serde_json::Value =
        serde_json::from_slice(&fs::read(candidate.join("record.json")).unwrap()).unwrap();
    assert_eq!(record["version"], 2);
    fs::remove_dir_all(root).unwrap();
}
