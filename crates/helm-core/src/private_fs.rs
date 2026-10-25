//! Private state files and directories: restrictive modes from creation, no symlink
//! following, and no adopting files that belong to someone else.

use std::{
    fs::{File, OpenOptions},
    io,
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::Path,
};

use nix::{fcntl::OFlag, unistd::geteuid};

fn unsafe_path(path: &Path, why: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("refusing to use {}: {why}", path.display()),
    )
}

/// Creates `path` (and missing parents) with mode 0700. An existing directory is left exactly
/// as it is: it may be a shared location such as `/tmp` or a home directory, and chmod-ing it
/// would be a surprise. Only a non-directory in the way is an error.
pub fn create_private_dir(path: &Path) -> io::Result<()> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    if !path.is_dir() {
        return Err(unsafe_path(path, "not a directory"));
    }
    Ok(())
}

/// Opens (creating with mode 0600) a regular file owned by the current user. Symlinks are not
/// followed. `append` selects append versus read/write without truncation.
pub fn open_private_file(path: &Path, append: bool) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options
        .create(true)
        .mode(0o600)
        .custom_flags(OFlag::O_NOFOLLOW.bits());
    if append {
        options.append(true);
    } else {
        options.read(true).write(true);
    }
    let file = options.open(path).map_err(|error| {
        if error.raw_os_error() == Some(nix::libc::ELOOP) {
            unsafe_path(path, "is a symbolic link")
        } else {
            error
        }
    })?;
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(unsafe_path(path, "not a regular file"));
    }
    if meta.uid() != geteuid().as_raw() {
        return Err(unsafe_path(path, "owned by another user"));
    }
    if meta.permissions().mode() & 0o077 != 0 {
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn creates_private_dirs_and_files_and_tightens_loose_existing_files() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("a/b");
        create_private_dir(&dir).unwrap();
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&root.path().join("a")), 0o700);
        // An existing directory keeps whatever mode it has.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        create_private_dir(&dir).unwrap();
        assert_eq!(mode(&dir), 0o755);
        let file = dir.join("helm.db");
        drop(open_private_file(&file, false).unwrap());
        assert_eq!(mode(&file), 0o600);
        // A pre-existing world-readable file (older install) is tightened, contents untouched.
        std::fs::write(&file, "data").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        drop(open_private_file(&file, false).unwrap());
        assert_eq!(mode(&file), 0o600);
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "data");
    }

    #[test]
    fn refuses_symlinks_without_touching_their_targets() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("victim");
        std::fs::write(&target, "keep").unwrap();
        let before = mode(&target);
        let link = root.path().join("helm.log");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let error = open_private_file(&link, true).unwrap_err();
        assert!(error.to_string().contains("symbolic link"), "{error}");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep");
        assert_eq!(mode(&target), before);
        let in_the_way = root.path().join("plain");
        std::fs::write(&in_the_way, "").unwrap();
        assert!(create_private_dir(&in_the_way).is_err());
    }
}
