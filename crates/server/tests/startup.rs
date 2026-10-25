use std::process::Command;
#[test]
fn server_rejects_missing_and_blank_api_key_before_touching_storage() {
    for key in [None, Some("  \n\t ")] {
        let dir = tempfile::tempdir().unwrap();
        let data = dir.path().join("must-not-be-created");
        let mut command = Command::new(env!("CARGO_BIN_EXE_transcript-vault"));
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
