use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;

fn command_with_config(config_file: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_sifter"));
    command.env("SIFTER_CONFIG_FILE", config_file);
    command
}

#[test]
fn collection_add_persists_yaml_config() {
    let temp = tempdir().expect("create tempdir");
    let config_file = temp.path().join("config.yml");
    let collection_root = temp.path().join("repo");
    fs::create_dir_all(&collection_root).expect("create collection root");

    command_with_config(&config_file)
        .args(["config", "collection", "add"])
        .arg(&collection_root)
        .args(["--name", "repo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"repo\""));

    let saved = fs::read_to_string(config_file).expect("read config");
    assert!(saved.contains("includeByDefault: true"));
    assert!(saved.contains("pattern:"));
    assert!(saved.contains("**/*"));
}

#[test]
fn collection_list_emits_known_collections_as_json() {
    let temp = tempdir().expect("create tempdir");
    let config_file = temp.path().join("config.yml");
    let collection_root = temp.path().join("repo");
    fs::create_dir_all(&collection_root).expect("create collection root");

    command_with_config(&config_file)
        .args(["config", "collection", "add"])
        .arg(&collection_root)
        .args(["--name", "repo"])
        .assert()
        .success();

    command_with_config(&config_file)
        .args(["config", "collection", "list", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"name\":\"repo\""));
}

#[test]
fn collection_can_be_renamed_and_removed() {
    let temp = tempdir().expect("create tempdir");
    let config_file = temp.path().join("config.yml");
    let collection_root = temp.path().join("repo");
    fs::create_dir_all(&collection_root).expect("create collection root");

    command_with_config(&config_file)
        .args(["config", "collection", "add"])
        .arg(&collection_root)
        .args(["--name", "repo"])
        .assert()
        .success();

    command_with_config(&config_file)
        .args(["config", "collection", "rename", "repo", "app"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"to\":\"app\""));

    command_with_config(&config_file)
        .args(["config", "collection", "list", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"name\":\"app\""))
        .stdout(predicate::str::contains("\"name\":\"repo\"").not());

    command_with_config(&config_file)
        .args(["config", "collection", "remove", "app"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"removed\":\"app\""));

    command_with_config(&config_file)
        .args(["config", "collection", "list", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"collections\":[]"));
}

#[test]
fn context_commands_add_list_check_and_remove_contexts() {
    let temp = tempdir().expect("create tempdir");
    let config_file = temp.path().join("config.yml");

    command_with_config(&config_file)
        .args([
            "config",
            "context",
            "add",
            "sifter://repo/src",
            "Source files",
        ])
        .assert()
        .success();

    command_with_config(&config_file)
        .args(["config", "context", "list", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Source files"));

    command_with_config(&config_file)
        .args([
            "config",
            "context",
            "check",
            "sifter://repo/src/lib.rs",
            "--json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"scope\":\"sifter://repo/src\""));

    command_with_config(&config_file)
        .args(["config", "context", "rm", "sifter://repo/src"])
        .assert()
        .success();

    command_with_config(&config_file)
        .args(["config", "context", "list", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"contexts\":[]"));
}
