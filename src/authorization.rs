use std::os::fd::AsRawFd;
use std::{
    env,
    ffi::{CString, OsString},
    fs::{self, File, Metadata, OpenOptions},
    io::{self, Write},
    os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt, fchown},
        process::ExitStatusExt,
    },
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
};

const HELPER_BASE: &str = "/usr/local/libexec/hm";
const POLICY_DIR: &str = "/private/etc/sudoers.d";
const MARKER: &str = "hm authorization v1\n";

unsafe extern "C" {
    fn acl_get_file(path: *const libc::c_char, kind: libc::c_int) -> *mut libc::c_void;
    fn acl_get_entry(
        acl: *mut libc::c_void,
        entry_id: libc::c_int,
        entry: *mut *mut libc::c_void,
    ) -> libc::c_int;
    fn acl_free(acl: *mut libc::c_void) -> libc::c_int;
}

fn has_extended_acl(path: &Path) -> Result<bool, String> {
    let name = CString::new(path.as_os_str().as_bytes()).map_err(|e| e.to_string())?;
    // SAFETY: name is a valid C path; the returned ACL is checked and freed below.
    let acl = unsafe { acl_get_file(name.as_ptr(), 0x100) }; // ACL_TYPE_EXTENDED
    if acl.is_null() {
        let error = io::Error::last_os_error();
        // macOS reports ENOENT when an existing object has no extended ACL.
        if error.raw_os_error() == Some(libc::ENOENT) && fs::symlink_metadata(path).is_ok() {
            return Ok(false);
        }
        return Err(format!(
            "Cannot inspect ACL on {}: {}",
            path.display(),
            error
        ));
    }
    let mut entry = std::ptr::null_mut();
    // SAFETY: acl is valid, and entry points to writable storage for ACL_FIRST_ENTRY.
    let result = unsafe { acl_get_entry(acl, 0, &mut entry) };
    let error = io::Error::last_os_error();
    // SAFETY: acl is an allocation returned by acl_get_file and is no longer used.
    unsafe {
        acl_free(acl);
    }
    if result == 0 {
        Ok(true)
    } else if error.raw_os_error() == Some(libc::EINVAL) {
        Ok(false)
    } else {
        Err(error.to_string())
    }
}

struct Installation {
    anchor: PathBuf,
    base: PathBuf,
    policies: PathBuf,
    owner: u32,
    group: u32,
    uid: u32,
}

impl Installation {
    fn system(uid: u32) -> Self {
        Self {
            anchor: "/".into(),
            base: HELPER_BASE.into(),
            policies: POLICY_DIR.into(),
            owner: 0,
            group: 0,
            uid,
        }
    }

    fn directory(&self) -> PathBuf {
        self.base.join(self.uid.to_string())
    }

    fn helper(&self) -> PathBuf {
        self.directory().join("hm")
    }

    fn policy(&self) -> PathBuf {
        self.policies.join(format!("hm-{}", self.uid))
    }

    fn rule(&self) -> String {
        let helper = self.helper();
        let path = helper.display();
        format!(
            "# Managed by hm for uid {}\n#{} ALL=(root) NOPASSWD: NOSETENV: {path} add *, {path} remove *, {path} toggle *, {path} restore\n",
            self.uid, self.uid,
        )
    }

    fn lock(&self) -> Result<File, String> {
        self.directory_tree(&self.base, true)?;
        let path = self.base.join(format!(".{}.lock", self.uid));
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&path)
            .map_err(|e| e.to_string())?;
        self.check_node(&path, false)?;
        // SAFETY: file owns a valid descriptor; the kernel releases this lock on exit.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err("Another authorization setup is running. Retry when it completes.".into());
        }
        Ok(file)
    }

    fn check_node(&self, path: &Path, directory: bool) -> Result<Metadata, String> {
        let meta = fs::symlink_metadata(path)
            .map_err(|e| format!("Cannot inspect {}: {e}", path.display()))?;
        if meta.uid() != self.owner
            || meta.mode() & 0o022 != 0
            || (directory && (!meta.is_dir() || meta.mode() & 0o001 == 0))
            || (!directory && (!meta.is_file() || meta.nlink() != 1))
            || has_extended_acl(path)?
        {
            return Err(format!(
                "Refusing unsafe authorization path: {}",
                path.display()
            ));
        }
        Ok(meta)
    }

    fn directory_tree(&self, path: &Path, create: bool) -> Result<(), String> {
        let relative = path.strip_prefix(&self.anchor).map_err(|e| e.to_string())?;
        let mut current = self.anchor.clone();
        self.check_node(&current, true)?;
        for part in relative.components() {
            current.push(part);
            if create {
                match fs::DirBuilder::new().mode(0o755).create(&current) {
                    Ok(()) => fs::set_permissions(&current, fs::Permissions::from_mode(0o755))
                        .map_err(|e| e.to_string())?,
                    Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
                    Err(e) => return Err(format!("Cannot create {}: {e}", current.display())),
                }
            }
            self.check_node(&current, true)?;
        }
        Ok(())
    }

    fn managed_file(&self, path: &Path, expected: Option<&str>) -> Result<(), String> {
        self.check_node(path, false)?;
        if let Some(expected) = expected {
            let actual = fs::read_to_string(path).map_err(|e| e.to_string())?;
            if actual != expected {
                return Err(format!(
                    "Refusing unmanaged authorization file: {}",
                    path.display()
                ));
            }
        }
        Ok(())
    }

    fn check_existing(&self) -> Result<(), String> {
        if fs::symlink_metadata(self.directory()).is_ok() {
            self.directory_tree(&self.directory(), false)?;
            self.managed_file(&self.directory().join(".managed"), Some(MARKER))?;
            for entry in fs::read_dir(self.directory()).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                if !["hm", ".managed", "source"]
                    .iter()
                    .any(|name| entry.file_name() == *name)
                {
                    return Err("Authorization directory contains unmanaged files.".into());
                }
                self.managed_file(&entry.path(), None)?;
            }
        }
        if fs::symlink_metadata(self.policy()).is_ok() {
            self.directory_tree(&self.policies, false)?;
            self.managed_file(&self.policy(), Some(&self.rule()))?;
        }
        Ok(())
    }

    fn stage(&self, parent: &Path, mode: u32) -> Result<tempfile::NamedTempFile, String> {
        let file = tempfile::Builder::new()
            .prefix(".hm-")
            .tempfile_in(parent)
            .map_err(|e| e.to_string())?;
        fchown(file.as_file(), Some(self.owner), Some(self.group)).map_err(|e| e.to_string())?;
        file.as_file()
            .set_permissions(fs::Permissions::from_mode(mode))
            .map_err(|e| e.to_string())?;
        Ok(file)
    }

    fn write_file(&self, path: &Path, contents: &[u8], mode: u32) -> Result<(), String> {
        let mut stage = self.stage(&self.base, mode)?;
        stage.write_all(contents).map_err(|e| e.to_string())?;
        stage.as_file().sync_all().map_err(|e| e.to_string())?;
        stage.persist(path).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn install(
        &self,
        source: &Path,
        installation_root: &Path,
        validate: impl Fn(&Path) -> Result<(), String>,
    ) -> Result<(), String> {
        self.check_existing()?;
        self.directory_tree(&self.directory(), true)?;
        self.directory_tree(&self.policies, true)?;
        self.write_file(&self.directory().join(".managed"), MARKER.as_bytes(), 0o644)?;
        let mut binary = self.stage(&self.base, 0o700)?;
        io::copy(
            &mut File::open(source).map_err(|e| e.to_string())?,
            binary.as_file_mut(),
        )
        .map_err(|e| e.to_string())?;
        binary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
        binary.as_file().sync_all().map_err(|e| e.to_string())?;
        let mut policy = self.stage(&self.policies, 0o440)?;
        policy
            .write_all(self.rule().as_bytes())
            .map_err(|e| e.to_string())?;
        policy.as_file().sync_all().map_err(|e| e.to_string())?;
        validate(policy.path())?;
        binary.persist(self.helper()).map_err(|e| e.to_string())?;
        self.write_file(
            &self.directory().join("source"),
            installation_root.as_os_str().as_encoded_bytes(),
            0o644,
        )?;
        policy.persist(self.policy()).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn remove(&self) -> Result<(), String> {
        self.check_existing()?;
        if self.policy().try_exists().map_err(|e| e.to_string())? {
            fs::remove_file(self.policy()).map_err(|e| e.to_string())?;
        }
        if self.directory().try_exists().map_err(|e| e.to_string())? {
            for name in ["hm", "source", ".managed"] {
                match fs::remove_file(self.directory().join(name)) {
                    Ok(()) => (),
                    Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                    Err(e) => return Err(e.to_string()),
                }
            }
            fs::remove_dir(self.directory()).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

fn current_executable() -> Result<PathBuf, String> {
    env::current_exe()
        .and_then(fs::canonicalize)
        .map_err(|e| format!("Cannot locate hm: {e}"))
}

fn sudo(program: &Path) -> Command {
    let mut command = Command::new(program);
    command.env_clear();
    for name in ["TERM", "NO_COLOR"] {
        if let Some(value) = env::var_os(name) {
            command.env(name, value);
        }
    }
    command
}

fn succeeded(status: ExitStatus) -> Result<(), String> {
    if status.success() {
        Ok(())
    } else {
        Err("Authorization was cancelled or failed; no hosts changes were made.".into())
    }
}

fn includes_policy_directory(contents: &str) -> bool {
    contents.lines().any(|line| {
        let mut words = line.split_whitespace();
        matches!(words.next(), Some("@includedir" | "#includedir"))
            && matches!(
                words.next(),
                Some("/private/etc/sudoers.d" | "/etc/sudoers.d")
            )
    })
}

fn check_setup_source(executable: &Path) -> Result<(), String> {
    // The password-free copy must never install executables or change sudoers.
    if executable.starts_with(HELPER_BASE)
        || fs::metadata(executable).map_err(|e| e.to_string())?.nlink() != 1
    {
        return Err("Run hm setup from your regular hm installation.".into());
    }
    Ok(())
}

fn check_hosts_files(
    installation: &Installation,
    hosts: &Path,
    restore: bool,
) -> Result<(), String> {
    installation.directory_tree(hosts.parent().expect("hosts parent"), false)?;
    installation.check_node(hosts, false)?;
    if restore {
        installation.check_node(&crate::storage::backup_path(hosts), false)?;
    }
    Ok(())
}

fn standard_etc_target(target: &Path) -> bool {
    target == Path::new("/private/etc") || target == Path::new("private/etc")
}

pub fn validate_system_hosts(restore: bool) -> Result<(), String> {
    let directory = Path::new("/private/etc");
    if !standard_etc_target(&fs::read_link("/etc").map_err(|e| e.to_string())?) {
        return Err("Authorized writes require the standard macOS /etc directory.".into());
    }
    check_hosts_files(&Installation::system(0), &directory.join("hosts"), restore)
}

pub fn setup(remove: bool) -> Result<String, String> {
    let executable = current_executable()?;
    check_setup_source(&executable)?;
    // SAFETY: geteuid only returns the calling process's effective user ID.
    if unsafe { libc::geteuid() } != 0 {
        eprintln!(
            "Administrator approval is required to {} password-free hosts changes for this user.",
            if remove { "revoke" } else { "enable" }
        );
        let mut command = sudo(Path::new("/usr/bin/sudo"));
        command.arg("--").arg(&executable).arg("setup");
        if remove {
            command.arg("--remove");
        }
        succeeded(
            command
                .status()
                .map_err(|e| format!("Cannot request administrator access: {e}"))?,
        )?;
        return Ok(String::new());
    }
    let uid = env::var("SUDO_UID")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .filter(|uid| *uid != 0)
        .ok_or("Run hm setup as your regular user.")?;
    let installation = Installation::system(uid);
    let _lock = installation.lock()?;
    if remove {
        installation.remove()?;
        return Ok("Password-free hosts changes disabled for this user.\n".into());
    }
    let sudoers = fs::read_to_string("/private/etc/sudoers").map_err(|e| e.to_string())?;
    if !includes_policy_directory(&sudoers) {
        return Err("The sudo policy must include /private/etc/sudoers.d; ask your administrator to configure it.".into());
    }
    let parent = executable.parent().expect("executable parent");
    let root = parent
        .parent()
        .filter(|root| root.join(".installer-managed").is_file())
        .unwrap_or(parent);
    installation.install(&executable, root, |policy| {
        let status = Command::new("/usr/sbin/visudo")
            .env_clear()
            .args(["-c", "-f"])
            .arg(policy)
            .status()
            .map_err(|e| e.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err("Authorization policy validation failed.".into())
        }
    })?;
    Ok("Password-free hosts changes enabled for this user.\n".into())
}

#[cfg(test)]
#[path = "authorization_tests.rs"]
mod tests;

pub fn execute(arguments: &[OsString]) -> Result<u8, String> {
    // SAFETY: getuid only returns the calling process's user ID.
    let installation = Installation::system(unsafe { libc::getuid() });
    execute_with(&installation, arguments, Path::new("/usr/bin/sudo"))
}

fn execute_with(
    installation: &Installation,
    arguments: &[OsString],
    sudo_path: &Path,
) -> Result<u8, String> {
    let not_ready =
        |error| format!("Authorization is unavailable: {error}. Run hm setup once, then retry.");
    installation
        .directory_tree(&installation.directory(), false)
        .map_err(not_ready)?;
    installation
        .directory_tree(&installation.policies, false)
        .map_err(not_ready)?;
    installation
        .managed_file(&installation.directory().join(".managed"), Some(MARKER))
        .map_err(not_ready)?;
    let metadata = installation
        .check_node(&installation.helper(), false)
        .map_err(not_ready)?;
    if metadata.mode() & 0o111 == 0 {
        return Err(not_ready("helper is not executable".into()));
    }
    // The root-owned sudoers fragment is not readable by a regular user.
    installation
        .check_node(&installation.policy(), false)
        .map_err(not_ready)?;
    let version = Command::new(installation.helper())
        .env_clear()
        .arg("--version")
        .output()
        .map_err(|e| not_ready(e.to_string()))?;
    if !version.status.success()
        || version.stdout != format!("{}\n", env!("CARGO_PKG_VERSION")).as_bytes()
    {
        return Err("Authorization needs an update. Run hm setup, then retry.".into());
    }
    let status = sudo(sudo_path)
        .args(["-n", "--"])
        .arg(installation.helper())
        .args(arguments)
        .status()
        .map_err(|e| format!("Cannot run the authorized helper: {e}"))?;
    Ok(status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(1)) as u8)
}
