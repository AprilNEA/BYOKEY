use serde_json::{Value, json};
use std::path::Path;
use std::process::{Command, Output};

fn inject(config: &Path, settings: &Path, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_byokey"))
        .args(["claude-code", "inject", "--config"])
        .arg(config)
        .arg("--settings")
        .arg(settings)
        .args(extra)
        .output()
        .unwrap()
}

fn read_json(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn copilot_setup_preserves_both_documents_and_is_repeatable() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("byokey.json");
    let settings = dir.path().join("claude.json");
    std::fs::write(&config, serde_json::to_vec(&json!({
        "port": 9018,
        "providers": {"claude": {"api_key": "preserve-provider-key"}, "gemini": {"enabled": false}},
        "future_setting": {"keep": [1, 2, 3]}
    })).unwrap()).unwrap();
    std::fs::write(&settings, serde_json::to_vec(&json!({
        "model": "my-selected-model",
        "permissions": {"allow": ["Read"]},
        "env": {"MY_VARIABLE": "keep", "ANTHROPIC_BASE_URL": "https://old.example", "CLAUDE_CODE_EFFORT_LEVEL": "max"}
    })).unwrap()).unwrap();

    let first = inject(&config, &settings, &["--backend", "copilot"]);
    assert_success(&first);
    let server = read_json(&config);
    assert_eq!(server["providers"]["claude"]["backend"], "copilot");
    assert_eq!(
        server["providers"]["claude"]["api_key"],
        "preserve-provider-key"
    );
    assert_eq!(server["providers"]["gemini"]["enabled"], false);
    assert_eq!(server["future_setting"]["keep"], json!([1, 2, 3]));
    let client = read_json(&settings);
    assert_eq!(client["model"], "my-selected-model");
    assert_eq!(client["permissions"]["allow"], json!(["Read"]));
    assert_eq!(client["env"]["MY_VARIABLE"], "keep");
    assert_eq!(client["env"]["CLAUDE_CODE_EFFORT_LEVEL"], "max");
    assert_eq!(client["env"]["ANTHROPIC_BASE_URL"], "http://127.0.0.1:9018");
    assert_eq!(client["env"]["ANTHROPIC_API_KEY"], "byokey-local");
    assert_eq!(client["env"]["CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS"], "1");
    assert!(client["env"].get("ANTHROPIC_MODEL").is_none());
    assert!(String::from_utf8_lossy(&first.stdout).contains("byokey restart --config"));

    let before_server = std::fs::read(&config).unwrap();
    let before_client = std::fs::read(&settings).unwrap();
    assert_success(&inject(&config, &settings, &["--backend", "copilot"]));
    assert_eq!(std::fs::read(&config).unwrap(), before_server);
    assert_eq!(std::fs::read(&settings).unwrap(), before_client);
}

#[test]
fn copilot_setup_creates_missing_files() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("server/settings.json");
    let settings = dir.path().join("client/settings.json");
    assert_success(&inject(&config, &settings, &["--backend", "copilot"]));
    assert_eq!(
        read_json(&config)["providers"]["claude"]["backend"],
        "copilot"
    );
    assert_eq!(
        read_json(&settings)["env"]["CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS"],
        "1"
    );
}

#[test]
fn yaml_backend_update_preserves_unknown_configuration() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("server.yaml");
    let settings = dir.path().join("client.json");
    std::fs::write(
        &config,
        "port: 9137\nproviders:\n  claude:\n    api_key: keep\ncustom:\n  nested: [one, two]\n",
    )
    .unwrap();
    assert_success(&inject(&config, &settings, &["--backend", "copilot"]));
    let server: Value = serde_yaml::from_slice(&std::fs::read(&config).unwrap()).unwrap();
    assert_eq!(server["providers"]["claude"]["backend"], "copilot");
    assert_eq!(server["providers"]["claude"]["api_key"], "keep");
    assert_eq!(server["custom"]["nested"], json!(["one", "two"]));
    assert_eq!(
        read_json(&settings)["env"]["ANTHROPIC_BASE_URL"],
        "http://127.0.0.1:9137"
    );
}

#[test]
fn invalid_client_settings_leave_both_files_untouched() {
    for invalid in ["{not json", "[]", "{\"env\":false}"] {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("server.json");
        let settings = dir.path().join("client.json");
        let original = b"{\"port\": 8123}";
        std::fs::write(&config, original).unwrap();
        std::fs::write(&settings, invalid).unwrap();
        assert!(
            !inject(&config, &settings, &["--backend", "copilot"])
                .status
                .success()
        );
        assert_eq!(std::fs::read(&config).unwrap(), original);
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), invalid);
    }
}

#[test]
fn invalid_server_configuration_is_not_silently_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("server.json");
    let settings = dir.path().join("client.json");
    std::fs::write(&config, "{broken").unwrap();
    std::fs::write(&settings, "{\"model\":\"keep\"}").unwrap();
    assert!(
        !inject(&config, &settings, &["--backend", "copilot"])
            .status
            .success()
    );
    assert_eq!(std::fs::read_to_string(&config).unwrap(), "{broken");
    assert_eq!(
        std::fs::read_to_string(&settings).unwrap(),
        "{\"model\":\"keep\"}"
    );
}

#[test]
fn plain_injection_changes_only_client_and_honors_url_override() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("server.json");
    let settings = dir.path().join("client.json");
    let original = br#"{"claude_code":{"settings":{"env":{"ANTHROPIC_BASE_URL":"https://configured.example","MY_VARIABLE":"configured"}}}}"#;
    std::fs::write(&config, original).unwrap();
    assert_success(&inject(
        &config,
        &settings,
        &["--url", "https://remote.example/proxy"],
    ));
    assert_eq!(std::fs::read(&config).unwrap(), original);
    let client = read_json(&settings);
    assert_eq!(
        client["env"]["ANTHROPIC_BASE_URL"],
        "https://remote.example/proxy"
    );
    assert_eq!(client["env"]["MY_VARIABLE"], "configured");
    assert!(
        client["env"]
            .get("CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS")
            .is_none()
    );
    assert_success(&inject(
        &config,
        &settings,
        &["--disable-experimental-betas"],
    ));
    assert_eq!(
        read_json(&settings)["env"]["CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS"],
        "1"
    );
}

#[test]
fn existing_copilot_backend_enables_compatibility_without_mutating_server() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("server.json");
    let settings = dir.path().join("client.json");
    let original = br#"{"providers":{"claude":{"backend":"copilot"}}}"#;
    std::fs::write(&config, original).unwrap();
    assert_success(&inject(&config, &settings, &[]));
    assert_eq!(std::fs::read(&config).unwrap(), original);
    assert_eq!(
        read_json(&settings)["env"]["CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS"],
        "1"
    );
}

#[test]
fn unsupported_backend_and_conflicting_options_fail_without_writes() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("server.json");
    let settings = dir.path().join("client.json");
    for extra in [
        vec!["--backend", "codex"],
        vec!["--backend", "copilot", "--url", "https://remote.example"],
    ] {
        assert!(!inject(&config, &settings, &extra).status.success());
        assert!(!config.exists());
        assert!(!settings.exists());
    }
}

#[test]
fn configured_auth_conflicts_are_preserved_and_reported_without_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("server.json");
    let settings = dir.path().join("client.json");
    std::fs::write(&config, "{}").unwrap();
    std::fs::write(&settings, r#"{"apiKeyHelper":"keep-helper-secret","env":{"ANTHROPIC_AUTH_TOKEN":"keep-token-secret"}}"#).unwrap();
    let output = inject(&config, &settings, &[]);
    assert_success(&output);
    let client = read_json(&settings);
    assert_eq!(client["apiKeyHelper"], "keep-helper-secret");
    assert_eq!(client["env"]["ANTHROPIC_AUTH_TOKEN"], "keep-token-secret");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("ANTHROPIC_AUTH_TOKEN"));
    assert!(stderr.contains("apiKeyHelper"));
    assert!(!stderr.contains("keep-token-secret"));
    assert!(!stderr.contains("keep-helper-secret"));
}

#[test]
fn same_file_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("settings.json");
    std::fs::write(&path, "{}").unwrap();
    assert!(
        !inject(&path, &path, &["--backend", "copilot"])
            .status
            .success()
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{}");
}

#[test]
fn aliases_of_the_same_missing_file_are_rejected_before_creating_directories() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("nested/settings.json");
    let settings = dir.path().join("missing/../nested/settings.json");
    let output = inject(&config, &settings, &["--backend", "copilot"]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("must be different files"));
    assert!(!dir.path().join("nested").exists());
    assert!(!dir.path().join("missing").exists());
}
