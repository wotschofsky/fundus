use std::process::Command;
#[test]
fn server_rejects_missing_and_blank_api_key_before_touching_storage() {
    for key in [None, Some("  \n\t ")] {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("must-not-be-created");
        let mut command = Command::new(env!("CARGO_BIN_EXE_fundus"));
        command
            .arg("serve")
            .env("VAULT_DATA_DIR", &data)
            .env_remove("ASSEMBLYAI_API_KEY");
        if let Some(key) = key {
            command.env("ASSEMBLYAI_API_KEY", key);
        }
        let output = command.output().unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("ASSEMBLYAI_API_KEY is required"));
        assert!(!data.exists());
    }
}

#[test]
fn proxy_mode_serves_only_with_access_settings_but_other_commands_run_without_them() {
    let dir = tempfile::tempdir().unwrap();
    let web = dir.path().join("web");
    std::fs::create_dir_all(&web).unwrap();
    std::fs::write(web.join("index.html"), "page").unwrap();
    let command = |data: &std::path::Path, args: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_fundus"));
        command
            .args(args)
            .env("VAULT_DATA_DIR", data)
            .env("VAULT_WEB_DIR", &web)
            .env("VAULT_AUTH_MODE", "proxy")
            .env("ASSEMBLYAI_API_KEY", "synthetic")
            .env_remove("VAULT_ACCESS_TEAM_DOMAIN")
            .env_remove("VAULT_ACCESS_AUD");
        command.output().unwrap()
    };
    let unserved = dir.path().join("must-not-be-created");
    let output = command(&unserved, &["serve"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("VAULT_ACCESS_TEAM_DOMAIN") && stderr.contains("VAULT_ACCESS_AUD"));
    assert!(!unserved.exists());
    // A backup checks no sign-in, so it needs no Access settings, e.g. while upgrading.
    let backup = dir.path().join("backup");
    let output = command(
        &dir.path().join("data"),
        &["backup", backup.to_str().unwrap()],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
