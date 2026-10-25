//! Reversible regular-file changes; backup bodies are sensitive until encrypted by a journal.

use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{CommandPreview, PreChangeBackup};

const MAX_FILE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct FileState {
    version: u32,
    existed: bool,
    mode: u32,
    uid: u32,
    gid: u32,
    original_hex: String,
    expected_after: String,
}

fn checksum(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn reject_extended_metadata(file: &File) -> Result<(), String> {
    // A zero-length query writes no memory; this function owns a valid file descriptor.
    let count = unsafe { nix::libc::flistxattr(file.as_raw_fd(), std::ptr::null_mut(), 0) };
    if count > 0 {
        return Err("files with extended attributes require manual editing; metadata preservation is unsupported".into());
    }
    if count < 0 && std::io::Error::last_os_error().raw_os_error() != Some(nix::libc::ENOTSUP) {
        return Err("cannot inspect extended file metadata".into());
    }
    Ok(())
}

type FileBytesAndMetadata = (Vec<u8>, u32, u32, u32);

fn regular_file_bytes(path: &Path) -> Result<Option<FileBytesAndMetadata>, String> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot safely read {}: {error}", path.display())),
    };
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.len() > MAX_FILE_BYTES as u64
        || metadata.permissions().mode() & 0o6000 != 0
    {
        return Err("reversible writes require a regular file with one link, at most 8 MiB".into());
    }
    let mut bytes = Vec::new();
    reject_extended_metadata(&file)?;
    file.take(MAX_FILE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err("file grew beyond backup limit".into());
    }
    Ok(Some((
        bytes,
        metadata.permissions().mode() & 0o7777,
        metadata.uid(),
        metadata.gid(),
    )))
}

pub fn prepare_file_change(preview: &CommandPreview) -> Result<Option<PreChangeBackup>, String> {
    prepare_file_change_inner(preview, None)
}

/// Trusted workspace callers hold the validated parent descriptor through preparation and apply.
/// Recovery records retain the original absolute path; backup reads use only this descriptor.
pub fn prepare_file_change_in_parent(
    preview: &CommandPreview,
    parent: &File,
) -> Result<Option<PreChangeBackup>, String> {
    prepare_file_change_inner(preview, Some(parent))
}

fn prepare_file_change_inner(
    preview: &CommandPreview,
    pinned_parent: Option<&File>,
) -> Result<Option<PreChangeBackup>, String> {
    if !matches!(preview.tool.as_str(), "fs_write" | "write") {
        return Ok(None);
    }
    let path = preview
        .input
        .get("path")
        .and_then(|v| v.as_str())
        .filter(|v| !v.is_empty())
        .ok_or("file path is required")?;
    let content = preview
        .input
        .get("content")
        .and_then(|v| v.as_str())
        .ok_or("file content must be a string")?;
    if content.len() > MAX_FILE_BYTES {
        return Err("file content exceeds 8 MiB".into());
    }
    let input = Path::new(path);
    if helm_core::is_protected_local_path(input) {
        return Err("protected HELM state cannot be edited by file tools".into());
    }
    let parent = input
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let target = if let Some(parent) = pinned_parent {
        if !input.is_absolute() {
            return Err("anchored writes require an absolute recovery path".into());
        }
        PathBuf::from(format!("/proc/self/fd/{}", parent.as_raw_fd()))
            .join(input.file_name().ok_or("file name required")?)
    } else {
        parent
            .canonicalize()
            .map_err(|e| e.to_string())?
            .join(input.file_name().ok_or("file name required")?)
    };
    let original = regular_file_bytes(&target)?;
    let (existed, original_bytes, mode, uid, gid) = match original {
        Some((bytes, mode, uid, gid)) => (true, bytes, mode, uid, gid),
        None => (
            false,
            vec![],
            0o600,
            nix::unistd::geteuid().as_raw(),
            nix::unistd::getegid().as_raw(),
        ),
    };
    let state = FileState {
        version: 1,
        existed,
        mode,
        uid,
        gid,
        original_hex: hex::encode(&original_bytes),
        expected_after: checksum(content.as_bytes()),
    };
    Ok(Some(PreChangeBackup {
        id: uuid::Uuid::new_v4().to_string(),
        file_path: if pinned_parent.is_some() {
            input
        } else {
            &target
        }
        .to_str()
        .ok_or("non-UTF-8 file paths are unsupported")?
        .to_owned(),
        checksum_before: checksum(&original_bytes),
        backup_content: serde_json::to_string(&state).map_err(|e| e.to_string())?,
        restored: false,
    }))
}

fn atomic_write(path: &Path, bytes: &[u8], state: &FileState) -> Result<(), String> {
    let parent = path.parent().ok_or("parent directory required")?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    reject_extended_metadata(temporary.as_file())?;
    temporary.write_all(bytes).map_err(|e| e.to_string())?;
    nix::unistd::fchown(
        temporary.as_file().as_raw_fd(),
        Some(nix::unistd::Uid::from_raw(state.uid)),
        Some(nix::unistd::Gid::from_raw(state.gid)),
    )
    .map_err(|e| format!("cannot preserve file ownership: {e}"))?;
    temporary
        .as_file()
        .set_permissions(fs::Permissions::from_mode(state.mode))
        .map_err(|e| e.to_string())?;
    temporary.as_file().sync_all().map_err(|e| e.to_string())?;
    temporary.persist(path).map_err(|e| e.error.to_string())?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|e| e.to_string())
}

pub fn apply_file_change(backup: &PreChangeBackup, content: &str) -> Result<(), String> {
    let state: FileState =
        serde_json::from_str(&backup.backup_content).map_err(|e| e.to_string())?;
    if state.version != 1 {
        return Err("unsupported recovery version".into());
    }
    let current = regular_file_bytes(Path::new(&backup.file_path))?;
    let unchanged = match current {
        Some((bytes, mode, uid, gid)) => {
            state.existed
                && checksum(&bytes) == backup.checksum_before
                && mode == state.mode
                && uid == state.uid
                && gid == state.gid
        }
        None => !state.existed,
    };
    if !unchanged {
        return Err("file changed after backup preparation; refusing overwrite".into());
    }
    if checksum(content.as_bytes()) != state.expected_after {
        return Err("file content changed after preparation".into());
    }
    atomic_write(Path::new(&backup.file_path), content.as_bytes(), &state)?;
    if !regular_file_bytes(Path::new(&backup.file_path))?.is_some_and(|(bytes, mode, uid, gid)| {
        checksum(&bytes) == state.expected_after
            && mode == state.mode
            && uid == state.uid
            && gid == state.gid
    }) {
        return Err("written file did not verify".into());
    }
    Ok(())
}

pub fn restore_file_change(path: &str, checksum_before: &str, body: &str) -> Result<(), String> {
    let state: FileState =
        serde_json::from_str(body).map_err(|e| format!("invalid recovery data: {e}"))?;
    if state.version != 1 {
        return Err("unsupported recovery version".into());
    }
    let original = hex::decode(&state.original_hex).map_err(|e| e.to_string())?;
    if checksum(&original) != checksum_before {
        return Err("backup checksum mismatch".into());
    }
    let target = PathBuf::from(path);
    if helm_core::is_protected_local_path(&target) {
        return Err("protected HELM state cannot be restored by file tools".into());
    }
    let current = regular_file_bytes(&target)?;
    match &current {
        Some((bytes, mode, uid, gid))
            if state.existed
                && checksum(bytes) == checksum_before
                && *mode == state.mode
                && *uid == state.uid
                && *gid == state.gid =>
        {
            return Ok(());
        }
        None if !state.existed => return Ok(()),
        Some((bytes, mode, uid, gid))
            if checksum(bytes) == state.expected_after
                && *mode == state.mode
                && *uid == state.uid
                && *gid == state.gid => {}
        _ => {
            return Err(
                "file no longer matches the applied change; refusing to overwrite newer work"
                    .into(),
            );
        }
    }
    if state.existed {
        atomic_write(&target, &original, &state)?;
    } else {
        fs::remove_file(&target).map_err(|e| e.to_string())?;
        File::open(target.parent().ok_or("parent directory required")?)
            .and_then(|directory| directory.sync_all())
            .map_err(|e| e.to_string())?;
    }
    let restored = regular_file_bytes(&target)?;
    if state.existed
        && !restored.is_some_and(|(bytes, mode, uid, gid)| {
            bytes == original && mode == state.mode && uid == state.uid && gid == state.gid
        })
    {
        return Err("restored file did not verify".into());
    }
    Ok(())
}
