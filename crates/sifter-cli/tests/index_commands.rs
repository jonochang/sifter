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
    let git_hooks = repo.join(".git/hooks");
    let target_dir = repo.join("target/debug");
    fs::create_dir_all(&docs).expect("create docs dir");
    fs::create_dir_all(&src).expect("create src dir");
    fs::create_dir_all(&git_hooks).expect("create .git hooks dir");
    fs::create_dir_all(&target_dir).expect("create target dir");
    let doc_path = docs.join("brief.md");
    let code_path = src.join("lib.rs");
    let client_path = src.join("client.rs");
    let options_path = src.join("options.rs");
    let notes_path = src.join("notes.rs");
    let git_sample_path = git_hooks.join("fsmonitor-watchman.sample");
    let target_artifact_path = target_dir.join("build.log");
    let ignored_path = src.join("ignored.rs");
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
        "use crate::{RequestOptions, RetryPolicy};\n\npub fn build(policy: RetryPolicy, options: RequestOptions) -> RetryPolicy { let _ = options.retries; policy }\n",
    )
    .expect("write related code");
    fs::write(
        &options_path,
        "pub struct RequestOptions { pub retries: usize }\n",
    )
    .expect("write dependency definition");
    fs::write(
        &notes_path,
        "// RetryPolicy is mentioned here, but this file does not depend on it.\n\
         pub fn note() -> &'static str { \"RetryPolicy\" }\n",
    )
    .expect("write unrelated note");
    fs::write(&repo.join(".gitignore"), "src/ignored.rs\n").expect("write .gitignore");
    fs::write(
        &ignored_path,
        "pub struct IgnoredType;\npub fn ignored_retry() -> usize { 1 }\n",
    )
    .expect("write ignored source");
    fs::write(&git_sample_path, "retry hook sample\n").expect("write .git sample");
    fs::write(&target_artifact_path, "retry artifact\n").expect("write target artifact");
    let canonical_doc_path = fs::canonicalize(&doc_path).unwrap_or_else(|_| doc_path.clone());
    let canonical_code_path = fs::canonicalize(&code_path).unwrap_or_else(|_| code_path.clone());
    let canonical_client_path =
        fs::canonicalize(&client_path).unwrap_or_else(|_| client_path.clone());
    let canonical_notes_path = fs::canonicalize(&notes_path).unwrap_or_else(|_| notes_path.clone());

    command_with_env(&config_file, &cache_home)
        .args(["config", "collection", "add"])
        .arg(&repo)
        .args(["--name", "repo"])
        .assert()
        .success();

    command_with_env(&config_file, &cache_home)
        .args(["config", "context", "global", "Workspace docs and code"])
        .assert()
        .success();

    command_with_env(&config_file, &cache_home)
        .args([
            "config",
            "context",
            "add",
            "sifter://repo/src",
            "Source files",
        ])
        .assert()
        .success();

    command_with_env(&config_file, &cache_home)
        .args(["index", "update", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"indexed_files\":5"));

    command_with_env(&config_file, &cache_home)
        .args(["index", "status", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"indexed_files\":5"))
        .stdout(predicate::str::contains("\"indexed_docs\":1"))
        .stdout(predicate::str::contains("\"indexed_code\":4"))
        .stdout(predicate::str::contains("\"vector_runtime\":\"pending\""));

    command_with_env(&config_file, &cache_home)
        .args(["search", "retry", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Retry Budget"))
        .stdout(predicate::str::contains("fsmonitor-watchman.sample").not())
        .stdout(predicate::str::contains("build.log").not())
        .stdout(predicate::str::contains("ignored.rs").not());

    command_with_env(&config_file, &cache_home)
        .args(["search", "rollout", "--docs", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"title\":\"Rollout\""))
        .stdout(predicate::str::contains("\"kind\":\"doc\""))
        .stdout(predicate::str::contains(
            "\"context\":\"Workspace docs and code\"",
        ));

    command_with_env(&config_file, &cache_home)
        .args(["search", "retry", "--code", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"kind\":\"code\""))
        .stdout(predicate::str::contains("lib.rs"))
        .stdout(predicate::str::contains("\"context\":\"Source files\""));

    command_with_env(&config_file, &cache_home)
        .args(["search", "--symbol", "RetryPolicy", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"name\":\"RetryPolicy\""))
        .stdout(predicate::str::contains("\"kind\":\"struct\""));

    command_with_env(&config_file, &cache_home)
        .args(["search", "--symbol", "RetryPolicy", "--defs", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            canonical_code_path.to_string_lossy().as_ref(),
        ))
        .stdout(predicate::str::contains(canonical_client_path.to_string_lossy().as_ref()).not())
        .stdout(predicate::str::contains("\"match_type\":\"definition\""));

    command_with_env(&config_file, &cache_home)
        .args(["search", "--symbol", "RetryPolicy", "--refs", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            canonical_client_path.to_string_lossy().as_ref(),
        ))
        .stdout(predicate::str::contains(canonical_code_path.to_string_lossy().as_ref()).not())
        .stdout(predicate::str::contains("\"match_type\":\"reference\""));

    command_with_env(&config_file, &cache_home)
        .args(["search", "--related"])
        .arg(&code_path)
        .args(["--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            canonical_client_path.to_string_lossy().as_ref(),
        ))
        .stdout(predicate::str::contains("\"score\":"))
        .stdout(predicate::str::contains(
            "\"shared_symbols\":[\"RetryPolicy\"]",
        ))
        .stdout(predicate::str::contains(canonical_notes_path.to_string_lossy().as_ref()).not());

    command_with_env(&config_file, &cache_home)
        .args(["search", "--related"])
        .arg(&client_path)
        .args(["--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("options.rs"))
        .stdout(predicate::str::contains("RequestOptions"));

    let get_output = command_with_env(&config_file, &cache_home)
        .args(["show"])
        .arg(&doc_path)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let get_json: Value = serde_json::from_slice(&get_output).expect("parse get output");
    assert_eq!(
        get_json["file"],
        canonical_doc_path.to_string_lossy().as_ref()
    );

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
        .args(["show", "sifter://repo/src/*.rs"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            canonical_code_path.to_string_lossy().as_ref(),
        ))
        .stdout(predicate::str::contains(
            canonical_client_path.to_string_lossy().as_ref(),
        ))
        .stdout(predicate::str::contains(
            canonical_notes_path.to_string_lossy().as_ref(),
        ));

    command_with_env(&config_file, &cache_home)
        .args(["search", "retry", "--files"])
        .assert()
        .success()
        .stdout(predicate::str::contains(
            canonical_doc_path.to_string_lossy().as_ref(),
        ))
        .stdout(predicate::str::contains(
            canonical_code_path.to_string_lossy().as_ref(),
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

    command_with_env(&config_file, &cache_home)
        .args(["search", "--symbol", "MissingSymbol", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("{\"results\":[]}"));

    command_with_env(&config_file, &cache_home)
        .args(["search", "--related", "sifter://repo/src/missing.rs", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("{\"results\":[]}"));
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

#[test]
fn update_warns_when_nothing_is_indexed() {
    let temp = tempdir().expect("create tempdir");
    let config_file = temp.path().join("config.yml");
    let cache_home = temp.path().join("cache");

    command_with_env(&config_file, &cache_home)
        .args(["index", "update", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"indexed_files\":0"))
        .stdout(predicate::str::contains("\"warning\":\"index is empty; check the collection path, mask, and ignore rules\""));
}
