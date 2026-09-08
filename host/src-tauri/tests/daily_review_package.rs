use std::fs;
use std::path::PathBuf;

#[test]
fn daily_review_reference_package_passes_host_inspection() {
    let package = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("reference-apps/daily-review/dist");
    let staging = std::env::temp_dir().join(format!(
        "kestral-daily-review-inspection-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging).unwrap();

    let result = host_lib::package::stage_and_inspect(&package, &staging);
    let _ = fs::remove_dir_all(&staging);

    let inspection =
        result.expect("Daily Review must satisfy Kestral's authoritative package contract");
    assert_eq!(inspection.id, "com.ma-zierl.daily-review");
}
