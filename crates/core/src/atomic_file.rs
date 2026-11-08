//! Writing a file so that a power loss never leaves it half-written.

use std::path::{Path, PathBuf};

/// Writes via a temporary file so a power loss never leaves a half-written file. A `private` file can be read by
/// its owner only (Unix).
pub fn write_atomically(path: &Path, text: &str, private: bool) -> std::io::Result<()> {
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory)?;
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    // The mode only applies to a new file; a leftover of an interrupted write keeps its own.
    let _ = std::fs::remove_file(&temporary);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = private;
    let mut file = options.open(&temporary)?;
    std::io::Write::write_all(&mut file, text.as_bytes())?;
    // Without the flush a power loss after the rename can leave an empty file under the final name.
    file.sync_all()?;
    drop(file);
    std::fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_the_file_and_leaves_no_temporary_one() {
        let directory = std::env::temp_dir().join(format!("freakymate-atomic-{}", std::process::id()));
        let path = directory.join("nested").join("file.conf");

        write_atomically(&path, "first", false).unwrap();
        write_atomically(&path, "second", false).unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
        assert!(!path.with_extension("conf.tmp").exists());
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
