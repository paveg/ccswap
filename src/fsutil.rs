use anyhow::{Context, Result};
use serde_json::Value;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn write_json_atomic(path: &Path, value: &Value, mode: Option<u32>) -> Result<()> {
    let mut bytes = serde_json::to_vec_pretty(value).context("serialize JSON")?;
    bytes.push(b'\n');
    write_bytes_atomic(path, &bytes, mode)
}

pub fn write_bytes_atomic(path: &Path, bytes: &[u8], mode: Option<u32>) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .context("path has no parent directory")?;
    fs::create_dir_all(parent).with_context(|| format!("create {}", parent.display()))?;

    let tmp = temp_path(path)?;
    let result = write_bytes_atomic_inner(path, &tmp, parent, bytes, mode);
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn write_bytes_atomic_inner(
    path: &Path,
    tmp: &Path,
    parent: &Path,
    bytes: &[u8],
    mode: Option<u32>,
) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);

    #[cfg(unix)]
    if let Some(mode) = mode {
        options.mode(mode);
    }

    let mut file = options
        .open(tmp)
        .with_context(|| format!("create temporary file {}", tmp.display()))?;
    file.write_all(bytes)
        .with_context(|| format!("write temporary file {}", tmp.display()))?;
    file.sync_all()
        .with_context(|| format!("sync temporary file {}", tmp.display()))?;
    drop(file);

    fs::rename(tmp, path).with_context(|| {
        format!(
            "rename temporary file {} to {}",
            tmp.display(),
            path.display()
        )
    })?;
    sync_parent(parent)?;
    Ok(())
}

fn temp_path(path: &Path) -> Result<PathBuf> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .context("path has no UTF-8 file name")?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?
        .as_nanos();
    Ok(path.with_file_name(format!(".{file_name}.{}.{}.tmp", std::process::id(), nanos)))
}

fn sync_parent(parent: &Path) -> Result<()> {
    let dir = File::open(parent).with_context(|| format!("open {}", parent.display()))?;
    dir.sync_all()
        .with_context(|| format!("sync {}", parent.display()))
}

#[cfg(unix)]
pub fn existing_mode(path: &Path, default_mode: u32) -> u32 {
    fs::metadata(path)
        .map(|metadata| metadata.permissions().mode() & 0o777)
        .unwrap_or(default_mode)
}

#[cfg(not(unix))]
pub fn existing_mode(_path: &Path, default_mode: u32) -> u32 {
    default_mode
}

pub fn remove_file_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_context(|| format!("remove {}", path.display())),
    }
}

pub fn ensure_private_dir(path: &Path) -> Result<()> {
    fs::create_dir_all(path).with_context(|| format!("create {}", path.display()))?;
    #[cfg(unix)]
    {
        let permissions = fs::Permissions::from_mode(0o700);
        fs::set_permissions(path, permissions)
            .with_context(|| format!("set permissions on {}", path.display()))?;
    }
    Ok(())
}
