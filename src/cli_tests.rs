use super::*;
use std::fs;

fn parse(args: &[&str]) -> Result<Cli, clap::Error> {
    Cli::try_parse_from(std::iter::once("hm").chain(args.iter().copied()))
}

fn cli(path: &Path, args: &[&str]) -> Result<String, String> {
    let options = parse(args).map_err(|error| error.to_string())?;
    run(options, path, &Renderer::new(80, false, false))
}

#[test]
fn accepts_subcommands_and_rejects_invalid_option_combinations() {
    for args in [
        vec!["list"],
        vec!["list", "--search", "local", "--json"],
        vec!["add", "::1", "app.local", "api.local", "--force"],
        vec!["add", "--force", "::1", "app.local"],
        vec!["remove", "app.local", "api.local"],
        vec!["toggle", "app.local", "api.local"],
        vec!["restore"],
        vec!["setup"],
        vec!["setup", "--remove"],
        vec!["--version"],
    ] {
        assert!(parse(&args).is_ok(), "{args:?}");
    }
    for name in ["list", "add", "remove", "toggle", "restore", "setup"] {
        assert_eq!(
            parse(&[name, "--help"]).err().unwrap().kind(),
            clap::error::ErrorKind::DisplayHelp,
            "{name}"
        );
    }
    for name in [
        "ls",
        "add",
        "remove",
        "rm",
        "delete",
        "toggle",
        "help",
        "version",
        "--list",
        "--add",
        "--remove",
        "--toggle",
        "--restore",
        "--ls",
        "--rm",
        "--delete",
        "--doctor",
        "--dry-run",
        "-h",
        "-V",
    ] {
        assert!(parse(&[name]).is_err(), "{name}");
    }
    for args in [
        vec!["add", "127.0.0.1"],
        vec!["list", "restore"],
        vec!["add", "::1", "app.local", "--remove", "localhost"],
        vec!["list", "--force"],
        vec!["add", "::1", "app.local", "--dry-run"],
        vec!["restore", "--json"],
        vec!["add", "::1", "app.local", "--search", "local"],
        vec!["--json"],
        vec!["--version", "restore"],
        vec!["list", "--version"],
        vec!["--json", "list"],
        vec!["--force", "add", "::1", "app.local"],
        vec!["list", "-s", "local"],
        vec!["list", "--ip", "127.0.0.1"],
        vec!["add", "::1", "app.local", "-f"],
        vec!["setup", "--force"],
        vec!["setup", "--remove", "example.com"],
    ] {
        assert!(parse(&args).is_err(), "{args:?}");
    }
}

#[test]
fn list_json_schema_search_and_empty_results() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hosts");
    fs::write(
        &path,
        "# heading\n127.0.0.1 app.local API.local\n# ::1 disabled.local\n",
    )
    .unwrap();
    let result = cli(&path, &["list", "--search", "api", "--json"]).unwrap();
    let json: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(
        json,
        serde_json::json!([{"ip":"127.0.0.1","hostnames":["app.local","API.local"],"active":true}])
    );
    assert_eq!(
        cli(&path, &["list", "--search", "missing", "--json"])
            .unwrap()
            .trim(),
        "[]"
    );
    assert!(
        cli(&path, &["list", "--search", "::1"])
            .unwrap()
            .contains("disabled.local")
    );
}

#[test]
fn full_lifecycle_preserves_rejected_requests() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hosts");
    let original = "127.0.0.1 localhost\n";
    fs::write(&path, original).unwrap();
    let result = cli(&path, &["add", "::1", "app.local", "api.local"]).unwrap();
    assert!(result.contains("Added:"));
    assert!(result.contains("Changes saved."));
    assert_eq!(
        fs::read_to_string(storage::backup_path(&path)).unwrap(),
        original
    );
    assert!(cli(&path, &["add", "::1", "app.local"]).is_err());
    cli(&path, &["add", "10.0.0.1", "app.local", "--force"]).unwrap();
    cli(&path, &["toggle", "app.local"]).unwrap();
    cli(&path, &["restore"]).unwrap();
    for host in ["app.local", "api.local"] {
        cli(&path, &["remove", host]).unwrap();
    }
    let before = fs::read(&path).unwrap();
    let backup = fs::read(storage::backup_path(&path)).unwrap();
    for args in [
        vec!["add", "bad-ip", "a"],
        vec!["add", "::1", "bad_name"],
        vec!["remove", "localhost", "missing"],
    ] {
        assert!(cli(&path, &args).is_err());
    }
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::read(storage::backup_path(&path)).unwrap(), backup);
}

#[test]
fn legacy_entries_and_comments_survive_command_lifecycle() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hosts");
    let original = "127.0.0.1 localhost app.local# keep  this note\n# ::1 legacy!name\n";
    fs::write(&path, original).unwrap();
    let listed = cli(&path, &["list", "--search", "app.local", "--json"]).unwrap();
    let entries: serde_json::Value = serde_json::from_str(&listed).unwrap();
    assert_eq!(
        entries[0]["hostnames"],
        serde_json::json!(["localhost", "app.local"])
    );
    cli(
        &path,
        &["add", "10.0.0.1", "app.local", "new.local", "--force"],
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(storage::backup_path(&path)).unwrap(),
        original
    );
    cli(&path, &["toggle", "app.local", "legacy!name"]).unwrap();
    let listed = cli(&path, &["list", "--json"]).unwrap();
    let entries: serde_json::Value = serde_json::from_str(&listed).unwrap();
    assert_eq!(entries[0]["ip"], "10.0.0.1");
    assert_eq!(entries[0]["active"], false);
    assert_eq!(entries[1]["hostnames"][0], "legacy!name");
    assert_eq!(entries[1]["active"], true);
    cli(&path, &["restore"]).unwrap();
    cli(&path, &["remove", "app.local", "legacy!name"]).unwrap();
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "10.0.0.1\tlocalhost # keep  this note\n\n10.0.0.1\tnew.local\n10.0.0.1\twww.new.local"
    );
}

#[test]
fn domain_pairs_share_a_command_lifecycle_and_preserve_backups() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hosts");
    let original = "127.0.0.1 localhost\n";
    fs::write(&path, original).unwrap();
    let added = cli(&path, &["add", "127.0.0.1", "www.example.com"]).unwrap();
    assert!(added.contains("www.example.com"));
    let entries = parse_lines(&storage::read(&path).unwrap().lines);
    assert_eq!(entries.len(), 3);
    for hostname in ["example.com", "www.example.com"] {
        assert!(
            entries
                .iter()
                .any(|entry| entry.hostnames[0] == hostname && entry.active)
        );
    }
    let paired = fs::read(&path).unwrap();
    cli(&path, &["toggle", "example.com"]).unwrap();
    assert_eq!(fs::read(storage::backup_path(&path)).unwrap(), paired);
    let entries = parse_lines(&storage::read(&path).unwrap().lines);
    assert!(
        entries
            .iter()
            .filter(|entry| entry.hostnames[0] != "localhost")
            .all(|entry| !entry.active)
    );
    let before = fs::read(&path).unwrap();
    let backup = fs::read(storage::backup_path(&path)).unwrap();
    for command in ["toggle", "remove"] {
        assert!(cli(&path, &[command, "www.example.com", "missing.com"]).is_err());
    }
    assert_eq!(fs::read(&path).unwrap(), before);
    assert_eq!(fs::read(storage::backup_path(&path)).unwrap(), backup);
    cli(&path, &["toggle", "www.example.com"]).unwrap();
    assert_eq!(fs::read(&path).unwrap(), paired);
    cli(&path, &["remove", "www.example.com"]).unwrap();
    let entries = parse_lines(&storage::read(&path).unwrap().lines);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].hostnames[0], "localhost");
    cli(&path, &["restore"]).unwrap();
    assert_eq!(fs::read(&path).unwrap(), paired);
}

#[test]
fn file_errors_are_reported_without_partial_results() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("missing");
    assert!(
        cli(&path, &["list"])
            .unwrap_err()
            .starts_with("Could not read")
    );
    assert_eq!(
        cli(&path, &["add", "bad-ip", "a"]).unwrap_err(),
        "Invalid IP address: bad-ip"
    );
}
