use super::*;
use std::os::unix::fs::symlink;

fn fixture() -> (tempfile::TempDir, Installation, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
    let meta = fs::metadata(dir.path()).unwrap();
    let installation = Installation {
        anchor: dir.path().into(),
        base: dir.path().join("libexec/hm"),
        policies: dir.path().join("sudoers.d"),
        owner: meta.uid(),
        group: meta.gid(),
        uid: 501,
    };
    let source = dir.path().join("client");
    fs::write(
        &source,
        format!(
            "#!/bin/bash\nprintf '%s\\n' '{}'\n",
            env!("CARGO_PKG_VERSION")
        ),
    )
    .unwrap();
    fs::set_permissions(&source, fs::Permissions::from_mode(0o755)).unwrap();
    (dir, installation, source)
}

fn install(installation: &Installation, source: &Path) {
    installation
        .install(source, &installation.anchor, |_| Ok(()))
        .unwrap();
}

#[test]
fn installs_updates_and_revokes_only_managed_files() {
    let (dir, installation, source) = fixture();
    let outside = dir.path().join("hosts");
    fs::write(&outside, "keep hosts and backups").unwrap();
    installation
        .install(&source, dir.path(), |path| {
            let result = Command::new("/usr/sbin/visudo")
                .args(["-c", "-f"])
                .arg(path)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            Ok(())
        })
        .unwrap();
    assert_eq!(
        fs::read(installation.helper()).unwrap(),
        fs::read(&source).unwrap()
    );
    assert_eq!(
        fs::read_to_string(installation.policy()).unwrap(),
        installation.rule()
    );
    assert_eq!(
        fs::metadata(installation.policy()).unwrap().mode() & 0o777,
        0o440
    );
    assert_eq!(
        fs::metadata(installation.helper()).unwrap().mode() & 0o777,
        0o755
    );
    assert_eq!(
        fs::metadata(installation.helper()).unwrap().uid(),
        installation.owner
    );
    let previous_inode = fs::metadata(installation.helper()).unwrap().ino();
    fs::write(&source, "new executable").unwrap();
    let other_root = dir.path().join("updated-source");
    installation
        .install(&source, &other_root, |_| Ok(()))
        .unwrap();
    assert_eq!(fs::read(installation.helper()).unwrap(), b"new executable");
    assert_ne!(
        fs::metadata(installation.helper()).unwrap().ino(),
        previous_inode
    );
    assert_eq!(
        fs::read(installation.directory().join("source")).unwrap(),
        other_root.as_os_str().as_encoded_bytes()
    );
    installation.remove().unwrap();
    installation.remove().unwrap();
    assert!(!installation.directory().exists());
    assert!(!installation.policy().exists());
    assert_eq!(
        fs::read_to_string(&outside).unwrap(),
        "keep hosts and backups"
    );
}

#[test]
fn invalid_policy_does_not_replace_the_existing_helper_or_policy() {
    let (_dir, installation, source) = fixture();
    install(&installation, &source);
    let helper = fs::read(installation.helper()).unwrap();
    let rule = fs::read(installation.policy()).unwrap();
    let inode = fs::metadata(installation.helper()).unwrap().ino();
    fs::write(&source, "replacement").unwrap();
    assert!(
        installation
            .install(&source, &installation.anchor, |_| Err(
                "invalid policy".into()
            ))
            .is_err()
    );
    assert_eq!(fs::read(installation.helper()).unwrap(), helper);
    assert_eq!(fs::read(installation.policy()).unwrap(), rule);
    assert_eq!(fs::metadata(installation.helper()).unwrap().ino(), inode);
    installation.check_existing().unwrap();
}

#[test]
fn refuses_symlinks_hard_links_writable_paths_and_wrong_owners() {
    for mode in [
        "parent-link",
        "helper-link",
        "hard-link",
        "directory-mode",
        "file-mode",
        "owner",
        "acl",
    ] {
        let (dir, mut installation, source) = fixture();
        let outside = dir.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep"), "untouched").unwrap();
        if mode == "parent-link" {
            fs::create_dir(installation.base.parent().unwrap()).unwrap();
            symlink(&outside, &installation.base).unwrap();
            assert!(
                installation
                    .install(&source, dir.path(), |_| Ok(()))
                    .is_err()
            );
        } else {
            install(&installation, &source);
            match mode {
                "helper-link" => {
                    fs::remove_file(installation.helper()).unwrap();
                    symlink(outside.join("keep"), installation.helper()).unwrap();
                }
                "hard-link" => {
                    fs::hard_link(installation.helper(), outside.join("linked-helper")).unwrap()
                }
                "directory-mode" => fs::set_permissions(
                    installation.base.clone(),
                    fs::Permissions::from_mode(0o775),
                )
                .unwrap(),
                "file-mode" => {
                    fs::set_permissions(installation.helper(), fs::Permissions::from_mode(0o777))
                        .unwrap()
                }
                "owner" => installation.owner += 1,
                "acl" => {
                    assert!(
                        Command::new("/bin/chmod")
                            .arg("+a")
                            .arg("everyone allow write")
                            .arg(installation.helper())
                            .status()
                            .unwrap()
                            .success()
                    );
                    assert_eq!(
                        fs::metadata(installation.helper()).unwrap().mode() & 0o022,
                        0
                    );
                }
                _ => unreachable!(),
            }
            assert!(
                installation
                    .install(&source, dir.path(), |_| Ok(()))
                    .is_err(),
                "{mode}"
            );
            assert!(installation.remove().is_err(), "{mode}");
            assert!(installation.policy().exists());
        }
        assert_eq!(
            fs::read_to_string(outside.join("keep")).unwrap(),
            "untouched"
        );
    }
}

#[test]
fn preserves_unmanaged_authorization_files() {
    for name in ["policy", ".managed", "unrelated"] {
        let (_dir, installation, source) = fixture();
        install(&installation, &source);
        let target = if name == "policy" {
            installation.policy()
        } else {
            installation.directory().join(name)
        };
        if target.exists() {
            fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
        }
        fs::write(&target, "unrelated configuration").unwrap();
        assert!(
            installation
                .install(&source, &installation.anchor, |_| Ok(()))
                .is_err()
        );
        assert!(installation.remove().is_err());
        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            "unrelated configuration"
        );
        assert!(installation.helper().exists());
    }
}

#[test]
fn delegates_without_a_shell_or_password_prompt_and_preserves_exit_status() {
    let (dir, installation, source) = fixture();
    install(&installation, &source);
    let log = dir.path().join("arguments");
    let sudo = dir.path().join("sudo");
    fs::write(&sudo, format!("#!/bin/bash\n[[ \"$1\" == -n && \"$2\" == -- ]] || exit 99\nshift 2\nprintf '%s\\0' \"$@\" > '{}'\nexit 23\n", log.display())).unwrap();
    fs::set_permissions(&sudo, fs::Permissions::from_mode(0o755)).unwrap();
    let injected = format!("$(touch {})", dir.path().join("injected").display());
    let args = vec![
        OsString::from("remove"),
        OsString::from(&injected),
        OsString::from("name with spaces"),
    ];
    assert_eq!(execute_with(&installation, &args, &sudo).unwrap(), 23);
    let bytes = fs::read(&log).unwrap();
    let received: Vec<_> = bytes
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .collect();
    assert_eq!(
        received,
        vec![
            installation.helper().as_os_str().as_encoded_bytes(),
            b"remove",
            injected.as_bytes(),
            b"name with spaces"
        ]
    );
    assert!(!dir.path().join("injected").exists());
    fs::set_permissions(installation.policy(), fs::Permissions::from_mode(0o000)).unwrap();
    assert_eq!(execute_with(&installation, &args, &sudo).unwrap(), 23);
    fs::write(&sudo, "#!/bin/bash\n/bin/kill -TERM $$\n").unwrap();
    assert_eq!(execute_with(&installation, &args, &sudo).unwrap(), 143);
}

#[test]
fn missing_or_outdated_authorization_never_invokes_sudo() {
    let (dir, installation, source) = fixture();
    let sudo = dir.path().join("must-not-run");
    assert!(
        execute_with(&installation, &[], &sudo)
            .unwrap_err()
            .contains("Run hm setup")
    );
    fs::write(&source, "#!/bin/bash\necho 0.0.0\n").unwrap();
    install(&installation, &source);
    assert!(
        execute_with(&installation, &[], &sudo)
            .unwrap_err()
            .contains("needs an update")
    );
    fs::write(
        &source,
        format!("#!/bin/bash\necho '{}'\n", env!("CARGO_PKG_VERSION")),
    )
    .unwrap();
    install(&installation, &source);
    fs::set_permissions(installation.policy(), fs::Permissions::from_mode(0o666)).unwrap();
    assert!(
        execute_with(&installation, &[], &sudo)
            .unwrap_err()
            .contains("unsafe")
    );
}

#[test]
fn privileged_and_hard_linked_copies_cannot_change_authorization() {
    assert!(check_setup_source(Path::new("/usr/local/libexec/hm/501/hm")).is_err());
    let (dir, _installation, source) = fixture();
    check_setup_source(&source).unwrap();
    fs::hard_link(&source, dir.path().join("linked-hm")).unwrap();
    assert!(check_setup_source(&source).is_err());
    assert!(check_setup_source(&dir.path().join("linked-hm")).is_err());
}

#[test]
fn recognizes_only_active_supported_sudoers_includes() {
    for text in [
        "@includedir /private/etc/sudoers.d\n",
        "  #includedir /etc/sudoers.d\n",
    ] {
        assert!(includes_policy_directory(text));
    }
    for text in [
        "# @includedir /private/etc/sudoers.d",
        "@includedir /tmp/policies",
        "@includedir /etc/sudoers.d-other",
        "",
    ] {
        assert!(!includes_policy_directory(text));
    }
}

#[test]
fn authorization_lock_is_exclusive_and_released_on_close() {
    let (_dir, installation, _source) = fixture();
    let lock = installation.lock().unwrap();
    assert!(installation.lock().is_err());
    drop(lock);
    installation.lock().unwrap();
}

#[test]
fn privileged_hosts_access_rejects_redirected_or_user_writable_files() {
    for target in ["hosts", "hosts.bak"] {
        for mode in ["symlink", "hard-link", "writable", "directory", "valid"] {
            let (dir, installation, _source) = fixture();
            let hosts = dir.path().join("hosts");
            let backup = dir.path().join("hosts.bak");
            let secret = dir.path().join("secret");
            fs::write(&hosts, "127.0.0.1 localhost\n").unwrap();
            fs::write(&backup, "saved hosts\n").unwrap();
            fs::write(&secret, "private data\n").unwrap();
            let path = dir.path().join(target);
            match mode {
                "symlink" => {
                    fs::remove_file(&path).unwrap();
                    symlink(&secret, &path).unwrap();
                }
                "hard-link" => {
                    fs::remove_file(&path).unwrap();
                    fs::hard_link(&secret, &path).unwrap();
                }
                "writable" => {
                    fs::set_permissions(&path, fs::Permissions::from_mode(0o666)).unwrap()
                }
                "directory" => {
                    fs::remove_file(&path).unwrap();
                    fs::create_dir(&path).unwrap();
                }
                "valid" => (),
                _ => unreachable!(),
            }
            assert_eq!(
                check_hosts_files(&installation, &hosts, true).is_ok(),
                mode == "valid",
                "{target}: {mode}"
            );
            if target == "hosts.bak" {
                check_hosts_files(&installation, &hosts, false).unwrap();
            }
            assert_eq!(fs::read_to_string(&secret).unwrap(), "private data\n");
        }
    }
}

#[test]
fn accepts_the_native_macos_etc_alias_and_rejects_other_targets() {
    assert!(standard_etc_target(&fs::read_link("/etc").unwrap()));
    for target in ["private/etc", "/private/etc"] {
        assert!(standard_etc_target(Path::new(target)));
    }
    for target in [
        "/tmp/etc",
        "../private/etc",
        "private/etc-other",
        "/private/etc/hosts",
    ] {
        assert!(!standard_etc_target(Path::new(target)));
    }
}
