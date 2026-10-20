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

#[test]
fn admin_credential_creation_is_explicit_and_persists_across_commands() {
    let dir = TempDir::new().unwrap();
    let database = dir.path().join("data.sqlite");
    let binary = env!("CARGO_BIN_EXE_terrarium-server");
    terrarium_server::store::Store::open(&database).unwrap();
    let token = |create: bool| {
        let mut command = Command::new(binary);
        command.args(["admin-token", "--database"]).arg(&database);
        if create {
            command.arg("--create");
        }
        command.output().unwrap()
    };
    assert!(!token(false).status.success());
    let created = token(true);
    assert!(created.status.success());
    let credential = String::from_utf8(created.stdout).unwrap();
    assert_eq!(credential.trim().len(), 64);
    assert!(credential.trim().bytes().all(|b| b.is_ascii_hexdigit()));
    assert_eq!(token(false).stdout, credential.as_bytes());
    assert_eq!(token(true).stdout, credential.as_bytes());
    let exported = dir.path().join("export");
    let result = Command::new(binary)
        .args(["export-legacy", "--database"])
        .arg(&database)
        .arg("--output")
        .arg(&exported)
        .output()
        .unwrap();
    assert!(result.status.success());
    assert_eq!(
        fs::read_to_string(exported.join("admin-token")).unwrap(),
        credential.trim()
    );
}

#[test]
fn invalid_private_listener_is_rejected_before_opening_database() {
    let dir = TempDir::new().unwrap();
    let database = dir.path().join("missing.sqlite");
    for (listen, cidr) in [
        ("0.0.0.0:8788", "192.168.0.0/16"),
        ("8.8.8.8:8788", "8.0.0.0/8"),
        ("127.0.0.1:8788", "192.168.0.0/16"),
        ("127.0.0.1:8788", "127.0.0.0/0"),
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_terrarium-server"))
            .args(["serve", "--database"])
            .arg(&database)
            .args(["--admin-listen", listen, "--admin-cidr", cidr])
            .env_remove("CORTICO_TERRARIUM_ADMIN_LISTEN")
            .env_remove("CORTICO_TERRARIUM_ADMIN_ALLOWED_CIDR")
            .output()
            .unwrap();
        assert!(!result.status.success());
        let error = String::from_utf8(result.stderr).unwrap();
        assert!(!error.contains("does not exist"), "{error}");
        assert!(!database.exists());
    }
}

#[cfg(unix)]
#[test]
fn native_controller_starts_writable_service_without_arguments_and_sigterm_stops_both_listeners() {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;
    let dir = TempDir::new().unwrap();
    terrarium_server::store::Store::open(dir.path().join("data/terrarium.sqlite")).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_terrarium-server"))
        .env("CORTICO_TERRARIUM_DEPLOY_ROOT", dir.path())
        .env("CORTICO_TERRARIUM_RELEASE_ID", "fixture-native")
        .env("PORT", "0")
        .env("CORTICO_TERRARIUM_ADMIN_HOST", "127.0.0.1")
        .env("CORTICO_TERRARIUM_ADMIN_PORT", "0")
        .env("CORTICO_TERRARIUM_ADMIN_ALLOWED_CIDR", "127.0.0.1/32")
        .env_remove("CORTICO_TERRARIUM_DATABASE")
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let mut private = String::new();
    output.read_line(&mut private).unwrap();
    assert!(
        private.starts_with("Private admin on 127.0.0.1:"),
        "{private}"
    );
    let mut public = String::new();
    output.read_line(&mut public).unwrap();
    assert!(public.ends_with("(writes: true)\n"), "{public}");
    for line in [&private, &public] {
        let address = line
            .split(" on ")
            .nth(1)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap();
        assert!(std::net::TcpStream::connect(address).is_ok());
    }
    assert!(
        Command::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    assert!(child.wait().unwrap().success());
    for line in [&private, &public] {
        let address = line
            .split(" on ")
            .nth(1)
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap();
        assert!(std::net::TcpStream::connect(address).is_err());
    }
}

#[test]
fn missing_validation_runtime_prevents_writable_listener_startup() {
    let dir = TempDir::new().unwrap();
    let database = dir.path().join("data.sqlite");
    terrarium_server::store::Store::open(&database).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_terrarium-server"))
        .args(["serve", "--database"])
        .arg(&database)
        .args(["--listen", "127.0.0.1:0", "--enable-writes"])
        .env(
            "CORTICO_TERRARIUM_LEGACY_NODE",
            dir.path().join("missing-node"),
        )
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("Legacy validator could not start")
    );
}
