use std::{fs, process::Command};
use tempfile::TempDir;
#[test]
fn dry_run_creates_no_database_and_missing_database_commands_fail() {
    let dir = TempDir::new().unwrap();
    let old = dir.path().join("old");
    fs::create_dir(&old).unwrap();
    fs::write(old.join("leaderboard.json"), "{}").unwrap();
    fs::write(old.join("credentials.json"), "{}").unwrap();
    let database = dir.path().join("data.sqlite");
    let binary = env!("CARGO_BIN_EXE_terrarium-server");
    let result = Command::new(binary)
        .args(["import-legacy", "--from"])
        .arg(&old)
        .arg("--database")
        .arg(&database)
        .output()
        .unwrap();
    assert!(result.status.success());
    assert!(
        String::from_utf8(result.stdout)
            .unwrap()
            .contains("dry run")
    );
    assert!(!database.exists());
    let result = Command::new(binary)
        .args(["check", "--database"])
        .arg(&database)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!database.exists());
    let result = Command::new(binary)
        .args(["serve", "--database"])
        .arg(&database)
        .args(["--listen", "0.0.0.0:8790"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!database.exists());
}
