use assert_cmd::Command;

#[test]
fn version_flag_prints_the_crate_version() {
    Command::cargo_bin("recollect")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(format!("recollect {}\n", env!("CARGO_PKG_VERSION")))
        .stderr("");
}
