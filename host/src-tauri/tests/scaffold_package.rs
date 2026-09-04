use std::path::PathBuf;

use host_lib::package;

#[test]
fn focused_app_scaffold_template_passes_host_package_inspection() {
    let package_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("templates/focused-app/src");

    let inspection = package::inspect(&package_root).expect("scaffold template must inspect");

    assert!(inspection.installable, "{:#?}", inspection.blocking_error);
    assert!(inspection.integrity_ok);
    assert!(inspection.host_compatible);
    assert_eq!(inspection.backend_kind, "none");
    assert_eq!(inspection.data.kind, "host-managed");
    assert_eq!(inspection.data.contract_version, Some(1));
    assert_eq!(inspection.surfaces.len(), 1);
    assert!(inspection.surfaces[0].has_custom_ui);
    assert_eq!(inspection.grant_requests.len(), 1);
}
