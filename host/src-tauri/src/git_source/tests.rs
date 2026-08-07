use super::*;

#[test]
fn public_url_rejects_non_https_and_credentials() {
    assert_eq!(
        validate_public_git_url("http://example.com/app.git").unwrap_err(),
        "public Git URLs must use HTTPS"
    );
    assert_eq!(
        validate_public_git_url("https://user:secret@example.com/app.git").unwrap_err(),
        "public Git URLs must not contain credentials"
    );
}

#[test]
fn public_url_rejects_loopback_hosts() {
    assert_eq!(
        validate_public_git_url("https://127.0.0.1/app.git").unwrap_err(),
        "Git URL host must resolve only to public network addresses"
    );
}

#[test]
fn public_address_rejects_mapped_loopback_and_special_ranges() {
    assert!(!is_public_ip("::ffff:127.0.0.1".parse().unwrap()));
    assert!(!is_public_ip("100.64.0.1".parse().unwrap()));
    assert!(!is_public_ip("198.18.0.1".parse().unwrap()));
    assert!(!is_public_ip("2001:db8::1".parse().unwrap()));
    assert!(is_public_ip("8.8.8.8".parse().unwrap()));
}

#[test]
fn archive_export_rejects_links() {
    let root = std::env::temp_dir().join(format!("git-export-test-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let archive_path = root.join("repository.tar");
    let file = File::create(&archive_path).unwrap();
    let mut builder = tar::Builder::new(file);
    let mut header = tar::Header::new_gnu();
    header.set_entry_type(tar::EntryType::Symlink);
    header.set_size(0);
    header.set_cksum();
    builder
        .append_link(&mut header, "app.json", "outside")
        .unwrap();
    builder.finish().unwrap();

    let error = extract_archive(&archive_path, &root.join("export")).unwrap_err();
    assert!(error.contains("unsupported non-regular entry"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn archive_export_ignores_pax_global_header() {
    let root = std::env::temp_dir().join(format!("git-export-test-{}", Uuid::new_v4()));
    fs::create_dir_all(&root).unwrap();
    let archive_path = root.join("repository.tar");
    let file = File::create(&archive_path).unwrap();
    let mut builder = tar::Builder::new(file);

    let pax_data = b"52 comment=0123456789012345678901234567890123456789\n";
    let mut pax_header = tar::Header::new_gnu();
    pax_header.set_entry_type(tar::EntryType::XGlobalHeader);
    pax_header.set_size(pax_data.len() as u64);
    pax_header.set_cksum();
    builder
        .append_data(&mut pax_header, "pax_global_header", &pax_data[..])
        .unwrap();

    let app_data = b"{}";
    let mut app_header = tar::Header::new_gnu();
    app_header.set_entry_type(tar::EntryType::Regular);
    app_header.set_size(app_data.len() as u64);
    app_header.set_cksum();
    builder
        .append_data(&mut app_header, "app.json", &app_data[..])
        .unwrap();
    builder.finish().unwrap();

    let export_root = root.join("export");
    extract_archive(&archive_path, &export_root).unwrap();
    assert_eq!(fs::read(export_root.join("app.json")).unwrap(), app_data);
    assert!(!export_root.join("pax_global_header").exists());
    let _ = fs::remove_dir_all(root);
}
