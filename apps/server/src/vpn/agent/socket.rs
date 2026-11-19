use std::{
    fs::{self, DirBuilder},
    io,
    os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt, chown},
    path::{Path, PathBuf},
};

use tokio::net::UnixListener;

pub(super) struct Socket {
    pub listener: UnixListener,
    path: PathBuf,
    device: u64,
    inode: u64,
}

impl Socket {
    pub fn bind(path: &Path, client_uid: u32) -> io::Result<Self> {
        let parent = path.parent().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "socket path has no parent")
        })?;
        let parent = prepare_directory(parent)?;
        let filename = path.file_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "socket path has no filename")
        })?;
        let path = parent.join(filename);
        let listener = UnixListener::bind(&path).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "could not bind VPN agent socket at {}: {error}; existing socket paths are never overwritten",
                    path.display()
                ),
            )
        })?;
        let metadata = fs::symlink_metadata(&path)?;
        let socket = Self {
            listener,
            path,
            device: metadata.dev(),
            inode: metadata.ino(),
        };
        fs::set_permissions(&socket.path, fs::Permissions::from_mode(0o600))?;
        chown(&socket.path, Some(client_uid), None)?;
        Ok(socket)
    }
}

fn prepare_directory(path: &Path) -> io::Result<PathBuf> {
    let directory = match fs::canonicalize(path) {
        Ok(path) => path,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let parent = path.parent().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "socket directory has no parent",
                )
            })?;
            let parent = fs::canonicalize(parent)?;
            validate_directory(&parent)?;
            let name = path.file_name().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "socket directory has no name")
            })?;
            let directory = parent.join(name);
            DirBuilder::new().mode(0o755).create(&directory)?;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o755))?;
            directory
        }
        Err(error) => return Err(error),
    };
    validate_directory(&directory)?;
    Ok(directory)
}

fn validate_directory(path: &Path) -> io::Result<()> {
    for directory in path.ancestors() {
        let metadata = fs::symlink_metadata(directory)?;
        if !metadata.is_dir() || metadata.uid() != 0 || metadata.permissions().mode() & 0o022 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "unsafe VPN agent socket directory {} (uid={}, mode={:04o}): must be root-owned and not group/world-writable",
                    directory.display(),
                    metadata.uid(),
                    metadata.permissions().mode() & 0o7777,
                ),
            ));
        }
    }
    Ok(())
}

impl Drop for Socket {
    fn drop(&mut self) {
        let Ok(metadata) = fs::symlink_metadata(&self.path) else {
            return;
        };
        if metadata.dev() == self.device && metadata.ino() == self.inode {
            let _ = fs::remove_file(&self.path);
        }
    }
}
