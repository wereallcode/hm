use std::process::{Command, Output};

fn cli(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_hm"))
        .args(args)
        .output()
        .unwrap()
}

fn stdout(result: &Output) -> String {
    String::from_utf8_lossy(&result.stdout).into()
}

#[test]
fn help_version_and_removed_features() {
    let out = cli(&["--help"]);
    assert!(out.status.success());
    assert!(stdout(&out).starts_with(&format!("hm — {}\n", env!("CARGO_PKG_DESCRIPTION"))));
    assert!(stdout(&out).contains("Usage:\n  hm list"));
    for name in ["add", "remove", "toggle", "list", "restore", "setup"] {
        assert!(stdout(&out).contains(&format!("hm {name}")));
        assert!(!stdout(&out).contains(&format!("hm --{name}")));
    }
    for removed in ["--doctor", "--dry-run", "Environment", "HM_PATH"] {
        assert!(!stdout(&out).contains(removed));
    }
    assert_eq!(
        stdout(&cli(&["--version"])).trim(),
        env!("CARGO_PKG_VERSION")
    );
    assert!(out.stderr.is_empty());
    let empty = cli(&[]);
    assert_eq!(empty.status.code(), Some(1));
    assert_eq!(String::from_utf8_lossy(&empty.stderr), stdout(&out));
    for name in ["list", "add", "remove", "toggle", "restore", "setup"] {
        let help = cli(&[name, "--help"]);
        assert!(help.status.success(), "{name}");
        assert_eq!(stdout(&help), stdout(&out));
        assert!(help.stderr.is_empty(), "{name}");
    }
    for args in [
        vec!["--"],
        vec!["unknown"],
        vec!["--doctor"],
        vec!["--dry-run"],
        vec!["--list"],
        vec!["--add", "127.0.0.1", "app.local"],
        vec!["--remove", "app.local"],
        vec!["--toggle", "app.local"],
        vec!["--restore"],
        vec!["add"],
        vec!["remove"],
        vec!["toggle"],
        vec!["list", "--unknown"],
    ] {
        assert_eq!(cli(&args).status.code(), Some(1));
    }
}

#[test]
fn parser_errors_keep_clap_details_and_use_common_layout() {
    for (args, argument) in [
        (vec!["--version", "list"], "--version"),
        (vec!["list", "--search"], "--search <QUERY>"),
        (vec!["--json"], "--json"),
        (vec!["add", "127.0.0.1"], "<HOSTNAME>"),
    ] {
        let result = cli(&args);
        assert_eq!(result.status.code(), Some(1));
        assert!(result.stdout.is_empty());
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(stderr.starts_with("Error: "), "{stderr}");
        assert!(stderr.contains(argument), "{stderr}");
        assert!(!stderr.contains("\\n"), "{stderr}");
        assert!(stderr.contains("Run hm --help"));
        assert!(!stderr.contains('\x1b'));
    }
}

#[test]
fn list_uses_system_file_even_if_old_environment_variable_is_set() {
    let result = Command::new(env!("CARGO_BIN_EXE_hm"))
        .env("HM_PATH", "/nonexistent-hm-test-file")
        .args(["list", "--json"])
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let entries: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert!(entries.is_array());
}
