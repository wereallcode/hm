use crate::mutations::Mutation;
use std::os::{
    fd::AsRawFd,
    unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt, fchown},
};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

pub struct Snapshot {
    pub lines: Vec<String>,
    pub eol: &'static str,
}

pub fn backup_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".bak");
    name.into()
}

fn lock(path: &Path) -> Result<fs::File, String> {
    let acquire = || -> std::io::Result<fs::File> {
        let target = fs::canonicalize(path)?;
        // Keep a separate persistent inode so atomic hosts replacements cannot bypass the lock.
        let mut name = std::ffi::OsString::from(".");
        name.push(target.file_name().expect("absolute file path"));
        name.push(".hm.lock");
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(target.with_file_name(name))?;
        let metadata = file.metadata()?;
        // SAFETY: geteuid only returns the calling process's effective user ID.
        if !metadata.is_file()
            || metadata.nlink() != 1
            || metadata.uid() != unsafe { libc::geteuid() }
            || metadata.mode() & 0o077 != 0
        {
            return Err(std::io::Error::other("Unsafe hosts lock file."));
        }
        loop {
            // SAFETY: file owns a valid descriptor; closing it releases the lock.
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
                return Ok(file);
            }
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::Interrupted {
                return Err(error);
            }
        }
    };
    acquire().map_err(|e| format!("Could not lock {}: {e}", path.display()))
}

pub fn read(path: &Path) -> Result<Snapshot, String> {
    let raw =
        fs::read_to_string(path).map_err(|e| format!("Could not read {}: {e}", path.display()))?;
    let eol = if raw.contains("\r\n") { "\r\n" } else { "\n" };
    Ok(Snapshot {
        lines: raw
            .replace("\r\n", "\n")
            .split('\n')
            .map(String::from)
            .collect(),
        eol,
    })
}

pub fn writable(path: &Path) -> Result<(), String> {
    fs::OpenOptions::new()
        .write(true)
        .open(path)
        .map(|_| ())
        .map_err(|e| {
            format!(
                "No write access to {}: {e}. Use sudo for system changes.",
                path.display()
            )
        })
}

pub fn write_atomic(path: &Path, raw: &[u8]) -> Result<(), String> {
    let write = || -> std::io::Result<()> {
        let target = fs::canonicalize(path)?;
        let metadata = fs::metadata(&target)?;
        let parent = target.parent().expect("absolute file path");
        let mut temporary = tempfile::Builder::new()
            .prefix(".hm-")
            .suffix(".tmp")
            .tempfile_in(parent)?;
        temporary.write_all(raw)?;
        let temporary_metadata = temporary.as_file().metadata()?;
        if temporary_metadata.uid() != metadata.uid() || temporary_metadata.gid() != metadata.gid()
        {
            fchown(
                temporary.as_file(),
                Some(metadata.uid()),
                Some(metadata.gid()),
            )?;
        }
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(metadata.mode()))?;
        temporary.as_file().sync_all()?;
        temporary.persist(target).map_err(|e| e.error)?;
        Ok(())
    };
    write().map_err(|e| format!("Write to {} failed: {e}", path.display()))
}

fn create_backup(path: &Path) -> Result<(), String> {
    let backup = backup_path(path);
    let copy = || -> std::io::Result<()> {
        let parent = backup
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let temporary = tempfile::Builder::new()
            .prefix(".hm-")
            .suffix(".tmp")
            .tempfile_in(parent)?;
        fs::copy(path, temporary.path())?;
        temporary.as_file().sync_all()?;
        temporary.persist(&backup).map_err(|e| e.error)?;
        Ok(())
    };
    copy().map_err(|e| format!("Could not create backup at {}: {e}", backup.display()))
}

pub fn update(
    path: &Path,
    mutate: impl FnOnce(&[String]) -> Result<Mutation, String>,
) -> Result<Mutation, String> {
    let _lock = lock(path)?;
    let snapshot = read(path)?;
    let result = mutate(&snapshot.lines)?;
    writable(path)?;
    create_backup(path)?;
    write_atomic(path, result.lines.join(snapshot.eol).as_bytes())?;
    Ok(result)
}

pub fn restore(path: &Path) -> Result<(), String> {
    let _lock = lock(path)?;
    let backup = backup_path(path);
    let raw = fs::read(&backup)
        .map_err(|e| format!("Could not read backup at {}: {e}", backup.display()))?;
    writable(path)?;
    write_atomic(path, &raw)
}
