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
        "# Retry Budget\n\nSifter should index retry budget design notes.\n\n## Rollout\n\nrollout checklist line 1\nrollout checklist line 2\n",
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
        .args(["config", "collection", "add"])
        .arg(&repo)
        .args(["--name", "repo"])
        .assert()
        .success();

    command_with_env(&config_file, &cache_home)
        .args(["index", "update", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"indexed_files\":3"));

    command_with_env(&config_file, &cache_home)
        .args(["index", "status", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"vector_runtime\":\"pending\""));

    command_with_env(&config_file, &cache_home)
        .args(["search", "retry", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Retry Budget"));

    command_with_env(&config_file, &cache_home)
        .args(["search", "rollout", "--docs", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"title\":\"Rollout\""))
        .stdout(predicate::str::contains("\"kind\":\"doc\""));

    command_with_env(&config_file, &cache_home)
        .args(["search", "retry", "--code", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"kind\":\"code\""))
        .stdout(predicate::str::contains("lib.rs"));

    command_with_env(&config_file, &cache_home)
        .args(["search", "--symbol", "RetryPolicy", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"name\":\"RetryPolicy\""))
        .stdout(predicate::str::contains("\"kind\":\"struct\""));

    command_with_env(&config_file, &cache_home)
        .args(["search", "--related"])
        .arg(&code_path)
        .args(["--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            client_path.to_string_lossy().as_ref(),
        ));

    let get_output = command_with_env(&config_file, &cache_home)
        .args(["show"])
        .arg(&doc_path)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let get_json: Value = serde_json::from_slice(&get_output).expect("parse get output");
    assert_eq!(get_json["file"], doc_path.to_string_lossy().as_ref());

    let docid = get_json["docid"].as_str().expect("docid").to_string();
    let virtual_path = get_json["virtual_path"]
        .as_str()
        .expect("virtual path")
        .to_string();

    command_with_env(&config_file, &cache_home)
        .args(["show", "--line-numbers", "-l", "2"])
        .arg(format!("{virtual_path}:5"))
        .assert()
        .success()
        .stdout(predicate::str::contains("   5: ## Rollout"))
        .stdout(predicate::str::contains("   6: "));

    command_with_env(&config_file, &cache_home)
        .args(["show"])
        .arg(format!("#{docid}"))
        .assert()
        .success()
        .stdout(predicate::str::contains("Retry Budget"));

    command_with_env(&config_file, &cache_home)
        .args(["show"])
        .arg(&doc_path)
        .arg(&code_path)
        .arg(&client_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("\"results\""));

    command_with_env(&config_file, &cache_home)
        .args(["search", "retry", "--files"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            doc_path.to_string_lossy().as_ref(),
        ))
        .stdout(predicate::str::contains(
            code_path.to_string_lossy().as_ref(),
        ));

    command_with_env(&config_file, &cache_home)
        .args(["search", "retry", "--csv"])
        .assert()
        .success()
        .stdout(predicate::str::contains("collection,context,docid,file"));

    command_with_env(&config_file, &cache_home)
        .args(["search", "retry", "--md"])
        .assert()
        .success()
        .stdout(predicate::str::contains("```json"));

    command_with_env(&config_file, &cache_home)
        .args(["search", "retry", "--xml"])
        .assert()
        .success()
        .stdout(predicate::str::contains("<results>"));
}

#[test]
fn semantic_and_hybrid_search_return_pending_runtime_error() {
    let temp = tempdir().expect("create tempdir");
    let config_file = temp.path().join("config.yml");
    let cache_home = temp.path().join("cache");

    for flag in ["--semantic", "--hybrid"] {
        let mut cmd = command_with_env(&config_file, &cache_home);
        cmd.arg("search");
        cmd.arg(flag);
        cmd.arg("retry budget")
            .assert()
            .code(2)
            .stdout(predicate::str::contains(
                "\"error\":\"vector_runtime_pending\"",
            ));
    }
}
