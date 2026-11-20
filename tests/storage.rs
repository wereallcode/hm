use hm::{mutations, storage::*};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt, symlink},
    path::Path,
};
fn add(path: &Path) -> Result<mutations::Mutation, String> {
    update(path, |lines| {
        mutations::add(lines, "10.0.0.1", &["app.local".into()], false)
    })
}

#[test]
fn rejected_requests_preserve_both_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hosts");
    fs::write(&path, "127.0.0.1 localhost\n").unwrap();
    fs::write(backup_path(&path), "old backup").unwrap();
    assert!(update(&path, |l| mutations::remove(l, &["missing".into()])).is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "127.0.0.1 localhost\n");
    assert_eq!(
        fs::read_to_string(backup_path(&path)).unwrap(),
        "old backup"
    );
    fs::remove_file(backup_path(&path)).unwrap();
    assert!(update(&path, |lines| mutations::remove(lines, &["missing".into()])).is_err());
    assert!(!backup_path(&path).exists());
}

#[test]
fn backups_are_exact_and_crlf_mode_and_owner_are_preserved() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hosts");
    let raw = "# Keep\r\n127.0.0.1 localhost alias\r\n";
    fs::write(&path, raw).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    let before = fs::metadata(&path).unwrap();
    update(&path, |l| mutations::remove(l, &["alias".into()])).unwrap();
    assert_eq!(fs::read_to_string(backup_path(&path)).unwrap(), raw);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        "# Keep\r\n127.0.0.1\tlocalhost\r\n"
    );
    let after = fs::metadata(&path).unwrap();
    assert_eq!(before.mode(), after.mode());
    assert_eq!(before.uid(), after.uid());
    assert_eq!(before.gid(), after.gid());
    assert_ne!(before.ino(), after.ino());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 3);
}

#[test]
fn symlink_survives_update_and_restore_with_backup_at_selected_path() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("hosts");
    let link = dir.path().join("link");
    let raw = "127.0.0.1 localhost\n";
    fs::write(&target, raw).unwrap();
    symlink(&target, &link).unwrap();
    add(&link).unwrap();
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(!backup_path(&target).exists());
    restore(&link).unwrap();
    assert_eq!(fs::read_to_string(&target).unwrap(), raw);
    assert_eq!(fs::read_to_string(backup_path(&link)).unwrap(), raw);
}

#[test]
fn replacing_linked_backups_preserves_their_targets() {
    for symbolic in [true, false] {
        for link_to_hosts in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("hosts");
            let outside = dir.path().join("outside");
            let original = "127.0.0.1 localhost\n";
            fs::write(&path, original).unwrap();
            fs::write(&outside, "keep this file\n").unwrap();
            let target = if link_to_hosts { &path } else { &outside };
            let backup = backup_path(&path);
            if symbolic {
                symlink(target, &backup).unwrap();
            } else {
                fs::hard_link(target, &backup).unwrap();
            }
            add(&path).unwrap();
            assert_eq!(fs::read_to_string(&outside).unwrap(), "keep this file\n");
            assert_eq!(fs::read_to_string(&backup).unwrap(), original);
            assert!(
                !fs::symlink_metadata(&backup)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            assert_ne!(
                fs::metadata(&backup).unwrap().ino(),
                fs::metadata(target).unwrap().ino()
            );
            restore(&path).unwrap();
            assert_eq!(fs::read_to_string(&path).unwrap(), original);
        }
    }
}

#[test]
fn failed_backup_prevents_write_and_read_failure_preserves_backup() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hosts");
    fs::write(&path, "127.0.0.1 localhost").unwrap();
    fs::create_dir(backup_path(&path)).unwrap();
    assert!(add(&path).is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "127.0.0.1 localhost");
    fs::remove_dir(backup_path(&path)).unwrap();
    fs::write(backup_path(&path), "saved").unwrap();
    fs::remove_file(&path).unwrap();
    assert!(add(&path).is_err());
    assert!(restore(&path).is_err());
    assert_eq!(fs::read_to_string(backup_path(&path)).unwrap(), "saved");
}

#[test]
fn restore_preserves_mixed_newlines_and_missing_final_newline() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hosts");
    fs::write(&path, "current").unwrap();
    let raw = "# saved\r\n127.0.0.1 localhost\n::1 localhost";
    assert!(restore(&path).is_err());
    fs::write(backup_path(&path), raw).unwrap();
    restore(&path).unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), raw);
    assert_eq!(fs::read_to_string(backup_path(&path)).unwrap(), raw);
}

#[test]
fn failed_atomic_write_leaves_no_temporary_file() {
    let dir = tempfile::tempdir().unwrap();
    assert!(write_atomic(dir.path(), b"replacement").is_err());
    assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
}

#[test]
fn read_only_file_rejects_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hosts");
    fs::write(&path, "127.0.0.1 localhost\n").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
    assert!(add(&path).is_err());
    assert!(!backup_path(&path).exists());
}

#[test]
fn concurrent_updates_share_a_lock_even_through_symlinks() {
    use std::{sync::mpsc, thread, time::Duration};
    for through_symlink in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts");
        fs::write(&path, "127.0.0.1 localhost\n").unwrap();
        let second_path = if through_symlink {
            let link = dir.path().join("alias");
            symlink(&path, &link).unwrap();
            link
        } else {
            path.clone()
        };
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let first_path = path.clone();
        let first = thread::spawn(move || {
            update(&first_path, |lines| {
                started_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                mutations::add(lines, "127.0.0.1", &["alpha".into()], false)
            })
            .unwrap();
        });
        started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let (entered_tx, entered_rx) = mpsc::channel();
        let second = thread::spawn(move || {
            update(&second_path, |lines| {
                entered_tx.send(()).unwrap();
                mutations::add(lines, "127.0.0.1", &["beta".into()], false)
            })
            .unwrap();
        });
        assert!(entered_rx.recv_timeout(Duration::from_millis(100)).is_err());
        release_tx.send(()).unwrap();
        first.join().unwrap();
        second.join().unwrap();
        let entries = hm::hosts::parse_lines(&read(&path).unwrap().lines);
        for name in ["alpha", "beta"] {
            assert!(
                entries
                    .iter()
                    .any(|e| e.hostnames.iter().any(|h| h == name))
            );
        }
    }
}

#[test]
fn restore_waits_for_an_update_before_reading_its_backup() {
    use std::{sync::mpsc, thread, time::Duration};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hosts");
    let original = "127.0.0.1 localhost\n";
    fs::write(&path, original).unwrap();
    fs::write(backup_path(&path), "older backup\n").unwrap();
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let first_path = path.clone();
    let first = thread::spawn(move || {
        update(&first_path, |lines| {
            started_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            mutations::add(lines, "127.0.0.1", &["alpha".into()], false)
        })
        .unwrap();
    });
    started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let restore_path = path.clone();
    let (restored_tx, restored_rx) = mpsc::channel();
    let restoring = thread::spawn(move || {
        restore(&restore_path).unwrap();
        restored_tx.send(()).unwrap();
    });
    assert!(
        restored_rx
            .recv_timeout(Duration::from_millis(100))
            .is_err()
    );
    release_tx.send(()).unwrap();
    first.join().unwrap();
    restoring.join().unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
    assert_eq!(fs::read_to_string(backup_path(&path)).unwrap(), original);
}

#[test]
fn unsafe_lock_files_are_rejected_without_changing_hosts() {
    for kind in ["symlink", "hard-link", "writable"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts");
        let outside = dir.path().join("outside");
        let lock = dir.path().join(".hosts.hm.lock");
        fs::write(&path, "127.0.0.1 localhost\n").unwrap();
        fs::write(&outside, "keep\n").unwrap();
        match kind {
            "symlink" => symlink(&outside, &lock).unwrap(),
            "hard-link" => fs::hard_link(&outside, &lock).unwrap(),
            "writable" => {
                fs::write(&lock, "").unwrap();
                fs::set_permissions(&lock, fs::Permissions::from_mode(0o666)).unwrap();
            }
            _ => unreachable!(),
        }
        assert!(add(&path).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "127.0.0.1 localhost\n");
        assert_eq!(fs::read_to_string(&outside).unwrap(), "keep\n");
        assert!(!backup_path(&path).exists());
    }
}
