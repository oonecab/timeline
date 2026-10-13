//! Locked, crash-safe snapshots. The core never observes a clock or filesystem.

use crate::{AuditEntry, Database, Error, IdempotencyEntry, Result};
use fs2::FileExt;
use serde_json::{Value, json};
use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Keys are scoped to a timeline and retained for seven days.
pub const IDEMPOTENCY_TTL_SECONDS: u64 = 7 * 24 * 60 * 60;

/// Resolve CLI > environment > per-user default without lossy path conversion.
pub fn resolve_db_path(explicit: Option<PathBuf>) -> Result<PathBuf> {
    resolve_path(explicit, std::env::var_os("TIMELINE_DB"), home_directory())
}

fn resolve_path(
    explicit: Option<PathBuf>,
    environment: Option<OsString>,
    home: Option<PathBuf>,
) -> Result<PathBuf> {
    let path = match explicit {
        Some(path) => path,
        None => match environment.filter(|value| !value.is_empty()) {
            Some(path) => PathBuf::from(path),
            None => home
                .ok_or_else(|| Error::new("cannot find a home directory; supply --db".into()))?
                .join(".timeline.json"),
        },
    };
    if path.as_os_str().is_empty() {
        return Err(Error::new("database path cannot be empty".into()));
    }
    Ok(path)
}

fn home_directory() -> Option<PathBuf> {
    #[cfg(windows)]
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
    #[cfg(not(windows))]
    let home = std::env::var_os("HOME");
    home.filter(|value| !value.is_empty()).map(PathBuf::from)
}

#[derive(Clone, Debug)]
pub struct Storage {
    path: PathBuf,
}

impl Storage {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// A stable sidecar is locked: replacing the DB inode must not replace its lock.
    fn lock(&self, exclusive: bool) -> Result<File> {
        let file = private_open(&sidecar(&self.path, ".lock"), false)
            .map_err(|_| Error::new("cannot open database lock".into()))?;
        if exclusive {
            FileExt::lock_exclusive(&file)
        } else {
            FileExt::lock_shared(&file)
        }
        .map_err(|_| Error::new("cannot lock database".into()))?;
        // Closing this handle releases the lock, including on early return/panic.
        Ok(file)
    }

    pub fn read(&self) -> Result<Database> {
        let _lock = self.lock(false)?;
        self.load().map(|(db, _)| db)
    }

    fn load(&self) -> Result<(Database, Option<Vec<u8>>)> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok((Database::default(), None));
            }
            Err(_) => return Err(Error::new("cannot read database".into())),
        };
        let mut db: Database = serde_json::from_slice(&bytes).map_err(|_| corrupt())?;
        if db.format_version != 1 {
            return Err(Error::new(
                "unsupported database format_version; restore a compatible .bak backup".into(),
            ));
        }
        db.rebuild_and_validate().map_err(|_| corrupt())?;
        Ok((db, Some(bytes)))
    }

    /// The entire read/modify/write sequence is one serialized transaction.
    /// Failed operations never change the DB, backup, counters or deduplication map.
    pub fn transaction<F>(
        &self,
        timeline_scope: &str,
        idempotency_key: Option<&str>,
        op: F,
    ) -> Result<Value>
    where
        F: FnOnce(&mut Database) -> Result<Value>,
    {
        let _lock = self.lock(true)?;
        let (mut db, previous) = self.load()?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Error::new("system clock precedes the Unix epoch".into()))?
            .as_secs();
        // A JSON pair is unambiguous even when keys contain separators or Unicode.
        let encoded_key = idempotency_key.map(|key| json!([timeline_scope, key]).to_string());
        if let Some(key) = &encoded_key {
            if let Some(entry) = db.idempotency.get(key) {
                if now.saturating_sub(entry.created_at) < IDEMPOTENCY_TTL_SECONDS {
                    return Ok(entry.result.clone());
                }
            }
        }
        let audit_start = db.audit.len();
        let previous_audit = serde_json::to_value(&db.audit)
            .map_err(|_| Error::new("cannot serialize audit history".into()))?;
        let result = op(&mut db)?;
        if db.audit.len() < audit_start
            || serde_json::to_value(&db.audit[..audit_start])
                .map_err(|_| Error::new("cannot serialize audit history".into()))?
                != previous_audit
        {
            return Err(Error::new("audit history is append-only".into()));
        }
        db.idempotency
            .retain(|_, entry| now.saturating_sub(entry.created_at) < IDEMPOTENCY_TTL_SECONDS);
        if let Some(encoded_key) = encoded_key {
            db.idempotency.insert(
                encoded_key,
                IdempotencyEntry {
                    created_at: now,
                    result: result.clone(),
                },
            );
            db.audit.push(AuditEntry {
                ts: 0,
                op: "idempotency_key".into(),
                timeline_id: timeline_scope.into(),
                from: None,
                to: None,
                node: result
                    .get("node")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                details: json!({"key": idempotency_key, "result": result}),
            });
        }
        let timestamp = now.max(db.audit[..audit_start].last().map_or(0, |entry| entry.ts));
        for entry in &mut db.audit[audit_start..] {
            entry.ts = timestamp;
        }
        db.rebuild_and_validate()?;
        let mut bytes = serde_json::to_vec_pretty(&db)
            .map_err(|_| Error::new("cannot serialize database".into()))?;
        bytes.push(b'\n');

        // Both snapshots use fsync + same-directory atomic replacement. Never
        // delete the destination first (that creates a missing/partial DB window).
        if let Some(previous) = previous {
            atomic_write(&sidecar(&self.path, ".bak"), &previous)?;
        }
        atomic_write(&self.path, &bytes)?;
        Ok(result)
    }
}

fn corrupt() -> Error {
    Error::new("database is corrupt; restore the .bak backup before retrying".into())
}

fn sidecar(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut temporary = tempfile::Builder::new()
        .prefix(".timeline-")
        .make_in(parent, |path| private_open(path, true))
        .map_err(|_| Error::new("cannot create a private database snapshot".into()))?;
    temporary
        .write_all(bytes)
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|_| Error::new("cannot flush database snapshot".into()))?;
    temporary
        .persist(path)
        .map_err(|_| Error::new("cannot atomically replace database snapshot".into()))?;
    // Windows persist uses MoveFileExW(REPLACE_EXISTING); Unix rename additionally
    // needs the containing directory synced for crash durability.
    #[cfg(unix)]
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| Error::new("cannot flush database directory".into()))?;
    Ok(())
}

#[cfg(unix)]
fn private_open(path: &Path, create_new: bool) -> io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(!create_new)
        .create_new(create_new)
        .mode(0o600)
        .open(path)
}

#[cfg(windows)]
fn private_open(path: &Path, create_new: bool) -> io::Result<File> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::FromRawHandle;
    use std::ptr::null_mut;
    use windows_sys::Win32::Foundation::{
        GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE, LocalFree,
    };
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::Storage::FileSystem::{
        CREATE_NEW, CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, OPEN_ALWAYS,
    };

    let mut name: Vec<u16> = path.as_os_str().encode_wide().collect();
    if name.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid filename",
        ));
    }
    name.push(0);
    // Protected DACL: only the object's owner has access; no parent ACEs inherit.
    // OW is the OWNER RIGHTS SID, which also works when an elevated user's token
    // sets Administrators as owner. Windows administrators retain OS-level rights.
    let security: Vec<u16> = "D:P(A;;FA;;;OW)\0".encode_utf16().collect();
    let mut descriptor = null_mut();
    // SAFETY: null-terminated UTF-16; descriptor is an OS-owned allocation freed
    // exactly once below, after CreateFileW has copied the security information.
    unsafe {
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            security.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            null_mut(),
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let handle = CreateFileW(
            name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            &attributes,
            if create_new { CREATE_NEW } else { OPEN_ALWAYS },
            FILE_ATTRIBUTE_NORMAL,
            null_mut(),
        );
        let error = (handle == INVALID_HANDLE_VALUE).then(io::Error::last_os_error);
        LocalFree(descriptor);
        if let Some(error) = error {
            Err(error)
        } else {
            Ok(File::from_raw_handle(handle))
        }
    }
}

#[cfg(not(any(unix, windows)))]
compile_error!("Timeline storage requires Unix file modes or Windows protected DACLs");

#[cfg(test)]
mod tests {
    use super::resolve_path;
    use std::ffi::OsString;
    use std::path::PathBuf;

    #[test]
    fn path_precedence_and_missing_home() {
        let explicit = Some(PathBuf::from("explicit.json"));
        let environment = Some(OsString::from("environment.json"));
        let home = Some(PathBuf::from("home"));
        assert_eq!(
            resolve_path(explicit, environment.clone(), home.clone()).unwrap(),
            PathBuf::from("explicit.json")
        );
        assert_eq!(
            resolve_path(None, environment, home.clone()).unwrap(),
            PathBuf::from("environment.json")
        );
        assert_eq!(
            resolve_path(None, None, home).unwrap(),
            PathBuf::from("home").join(".timeline.json")
        );
        assert!(resolve_path(None, None, None).is_err());
        assert!(resolve_path(Some(PathBuf::new()), None, None).is_err());
    }
}
