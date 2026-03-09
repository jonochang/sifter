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
