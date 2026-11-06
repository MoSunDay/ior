//! Private staging for raw WAV bytes handed to the local ASR CLI.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

const ATTEMPTS: u32 = 100;

/// Writes `bytes` to a new process-unique file under the system temp dir.
/// On Unix the file is created with mode 0600; `create_new` refuses to
/// follow a pre-existing file or symlink.
pub fn write(bytes: &[u8]) -> io::Result<PathBuf> {
    static SEQ: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir();
    for _ in 0..ATTEMPTS {
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!("ior-sensevoice-{}-{seq}.wav", std::process::id()));
        match write_new(&path, bytes) {
            Ok(()) => return Ok(path),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
            Err(err) => return Err(err),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not create a unique temp wav",
    ))
}

fn write_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    if let Err(err) = file.write_all(bytes) {
        drop(file);
        let _ = fs::remove_file(path);
        return Err(err);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    static SEQ: AtomicU32 = AtomicU32::new(0);

    fn temp_path(tag: &str) -> PathBuf {
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "ior-temp-wav-test-{tag}-{}-{seq}",
            std::process::id()
        ))
    }

    #[test]
    fn writes_exact_private_file() {
        let path = temp_path("private");
        write_new(&path, b"RIFF-stub").expect("write wav");
        assert_eq!(fs::read(&path).expect("read wav"), b"RIFF-stub");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).expect("metadata").permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        fs::remove_file(path).expect("remove wav");
    }

    #[test]
    fn refuses_existing_files_and_symlinks() {
        let path = temp_path("exists");
        fs::write(&path, b"keep").expect("seed file");
        let err = write_new(&path, b"new").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&path).expect("read seed"), b"keep");
        fs::remove_file(&path).expect("remove seed");

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let target = temp_path("target");
            let link = temp_path("link");
            fs::write(&target, b"secret").expect("seed target");
            symlink(&target, &link).expect("symlink");
            let err = write_new(&link, b"new").unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
            assert_eq!(fs::read(&target).expect("read target"), b"secret");
            fs::remove_file(link).expect("remove link");
            fs::remove_file(target).expect("remove target");
        }
    }
}
