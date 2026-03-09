use std::fs;

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;
use tempfile::tempdir;

fn command_with_env(config_file: &std::path::Path, cache_home: &std::path::Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_sifter"));
    command
        .env("SIFTER_CONFIG_FILE", config_file)
        .env("SIFTER_CACHE_HOME", cache_home);
    command
}

#[test]
fn update_status_search_get_and_multi_get_work_for_docs_and_code() {
    let temp = tempdir().expect("create tempdir");
    let config_file = temp.path().join("config.yml");
    let cache_home = temp.path().join("cache");
    let repo = temp.path().join("repo");
    let docs = repo.join("docs");
    let src = repo.join("src");
    fs::create_dir_all(&docs).expect("create docs dir");
    fs::create_dir_all(&src).expect("create src dir");
    let doc_path = docs.join("brief.md");
    let code_path = src.join("lib.rs");
    let client_path = src.join("client.rs");
    fs::write(
        &doc_path,
        "# Retry Budget\n\nSifter should index retry budget design notes.\n",
    )
    .expect("write doc");
    fs::write(
        &code_path,
        "pub struct RetryPolicy;\n\npub fn retry_budget() -> usize { 3 }\n",
    )
    .expect("write code");
    fs::write(
        &client_path,
        "use crate::RetryPolicy;\n\npub fn build(policy: RetryPolicy) -> RetryPolicy { policy }\n",
    )
    .expect("write related code");

    command_with_env(&config_file, &cache_home)
        .args(["collection", "add"])
        .arg(&repo)
        .args(["--name", "repo"])
        .assert()
        .success();

    command_with_env(&config_file, &cache_home)
        .args(["update", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"indexed_files\":3"));

    command_with_env(&config_file, &cache_home)
        .args(["status", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"vector_runtime\":\"pending\""));

    command_with_env(&config_file, &cache_home)
        .args(["search", "retry", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Retry Budget"));

    command_with_env(&config_file, &cache_home)
        .args(["symbol", "RetryPolicy", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"name\":\"RetryPolicy\""))
        .stdout(predicate::str::contains("\"kind\":\"struct\""));

    command_with_env(&config_file, &cache_home)
        .args(["related"])
        .arg(&code_path)
        .args(["--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            client_path.to_string_lossy().as_ref(),
        ));

    let get_output = command_with_env(&config_file, &cache_home)
        .args(["get"])
        .arg(&doc_path)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let get_json: Value = serde_json::from_slice(&get_output).expect("parse get output");
    assert_eq!(get_json["file"], doc_path.to_string_lossy().as_ref());

    command_with_env(&config_file, &cache_home)
        .args(["multi-get"])
        .arg(format!(
            "{},{},{}",
            doc_path.display(),
            code_path.display(),
            client_path.display()
        ))
        .assert()
        .success()
        .stdout(predicate::str::contains("\"results\""));
}

#[test]
fn vector_commands_return_pending_runtime_error() {
    let temp = tempdir().expect("create tempdir");
    let config_file = temp.path().join("config.yml");
    let cache_home = temp.path().join("cache");

    for command in ["embed", "vsearch", "query"] {
        command_with_env(&config_file, &cache_home)
            .arg(command)
            .arg("retry budget")
            .assert()
            .code(2)
            .stdout(predicate::str::contains(
                "\"error\":\"vector_runtime_pending\"",
            ));
    }
}
