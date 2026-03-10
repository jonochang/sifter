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
        .stdout(predicate::str::contains("config"))
        .stdout(predicate::str::contains("index"))
        .stdout(predicate::str::contains("search"))
        .stdout(predicate::str::contains("show"))
        .stdout(predicate::str::contains("collection").not())
        .stdout(predicate::str::contains("update").not())
        .stdout(predicate::str::contains("status").not());
}
