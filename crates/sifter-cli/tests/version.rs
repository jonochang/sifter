use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn prints_binary_name_in_version_output() {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_sifter"));

    cmd.arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("sifter"));
}

#[test]
fn help_shows_reduced_top_level_command_set() {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_sifter"));

    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Manage collections and scoped context"))
        .stdout(predicate::str::contains("Search docs, code, symbols, or related files"))
        .stdout(predicate::str::contains("config"))
        .stdout(predicate::str::contains("index"))
        .stdout(predicate::str::contains("search"))
        .stdout(predicate::str::contains("show"))
        .stdout(predicate::str::contains("update").not())
        .stdout(predicate::str::contains("status").not());
}

#[test]
fn search_help_describes_core_flags() {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_sifter"));

    cmd.args(["search", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Search indexed symbols by exact or prefix name."))
        .stdout(predicate::str::contains("Limit symbol search to definitions."))
        .stdout(predicate::str::contains("Find files related to an indexed file or virtual path."))
        .stdout(predicate::str::contains("Print only matching file paths."));
}
