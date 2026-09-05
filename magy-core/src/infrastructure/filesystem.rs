// Copyright (C) 2026 Zachary Joubert
//
// This file is part of Magy.
//
// Magy is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// Magy is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with Magy. If not, see <https://www.gnu.org/licenses/>.

use std::fs;
use std::path::{Path, PathBuf};
use tracing::{debug, error};
use crate::boundary::{is_within_boundary, validate_boundary};
use crate::Error;

use serde::{Deserialize, Serialize};

/// A directory entry representation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

/// Safely reads a file within the project boundary.
pub fn read_file(root: &Path, path: &Path) -> Result<String, Error> {
    let full_path = resolve_and_validate(root, path)?;

    debug!(path = ?full_path, "Reading file");
    fs::read_to_string(full_path).map_err(|e| {
        error!(error = ?e, "Read error");
        match e.kind() {
            std::io::ErrorKind::NotFound => Error::FileNotFound,
            _ => Error::Io,
        }
    })
}

/// Safely writes a file within the project boundary.
pub fn write_file(root: &Path, path: &Path, content: &str) -> Result<(), Error> {
    let full_path = resolve_and_validate(root, path)?;

    debug!(path = ?full_path, "Writing file");
    if let Some(parent) = full_path.parent() {
        if !parent.exists() {
             debug!(parent = ?parent, "Creating missing parent directories");
             fs::create_dir_all(parent).map_err(|e| {
                 error!(error = ?e, "Dir creation error");
                 Error::Io
             })?;
        }
    }

    fs::write(full_path, content).map_err(|e| {
        error!(error = ?e, "Write error");
        Error::Io
    })
}

/// Safely lists the contents of a directory within the project boundary.
pub fn list_directory(root: &Path, path: &Path) -> Result<Vec<DirEntry>, Error> {
    let full_path = resolve_and_validate(root, path)?;

    debug!(path = ?full_path, "Listing directory");

    if !full_path.is_dir() {
        debug!(path = ?full_path, "Not a directory");
        return Err(Error::Io);
    }

    let mut entries = Vec::new();
    let read_dir = fs::read_dir(full_path).map_err(|e| {
        error!(error = ?e, "Read dir error");
        Error::Io
    })?;

    for entry in read_dir {
        let entry = entry.map_err(|_| Error::Io)?;
        let file_type = entry.file_type().map_err(|_| Error::Io)?;

        entries.push(DirEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            is_dir: file_type.is_dir(),
        });
    }

    entries.sort();
    Ok(entries)
}

/// Recursively discovers all files and directories within the project boundary.
///
/// Returns a sorted list of paths relative to the project root.
pub fn discover_files(root: &Path) -> Result<Vec<PathBuf>, Error> {
    // 1. Canonicalize root once.
    let root_c = root.canonicalize().map_err(|_| Error::Io)?;

    let mut results = Vec::new();
    let mut stack = vec![root_c.clone()];

    while let Some(current_dir) = stack.pop() {
        let read_dir = fs::read_dir(&current_dir).map_err(|_| Error::Io)?;

        for entry in read_dir {
            let entry = entry.map_err(|_| Error::Io)?;
            let path = entry.path();
            let ft = entry.file_type().map_err(|_| Error::Io)?;

            // 2. Validate boundary (especially for symlinks/reparse points).
            validate_boundary(&root_c, &path)?;

            // 3. Construct relative path.
            let rel_path = path.strip_prefix(&root_c).map_err(|_| Error::Io)?.to_path_buf();
            results.push(rel_path);

            // 4. Recurse if it's a plain directory (not a symlink/reparse point).
            if ft.is_dir() && !is_link_or_reparse(&path, &ft)? {
                stack.push(path);
            }
        }
    }

    results.sort();
    Ok(results)
}

#[cfg(unix)]
fn is_link_or_reparse(_path: &Path, ft: &std::fs::FileType) -> Result<bool, Error> {
    Ok(ft.is_symlink())
}

#[cfg(windows)]
fn is_link_or_reparse(path: &Path, ft: &std::fs::FileType) -> Result<bool, Error> {
    if ft.is_symlink() {
        return Ok(true);
    }
    // Detect other reparse points (like junctions) via metadata attributes.
    // An inability to determine the type results in Error::Io to prevent unsafe recursion.
    let md = std::fs::symlink_metadata(path).map_err(|_| Error::Io)?;
    use std::os::windows::fs::MetadataExt;
    Ok((md.file_attributes() & 0x400) != 0)
}

fn resolve_and_validate(root: &Path, path: &Path) -> Result<PathBuf, Error> {
    let full_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };

    if !is_within_boundary(root, &full_path) {
        debug!(path = ?full_path, "Boundary violation");
        return Err(Error::OutsideBoundary);
    }

    Ok(full_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use std::env;
    use std::sync::Mutex;

    // Mutex to prevent concurrent CWD changes in tests
    static CWD_MUTEX: Mutex<()> = Mutex::new(());

    struct CwdGuard(PathBuf);
    impl CwdGuard {
        fn new() -> Self {
            Self(env::current_dir().unwrap())
        }
    }
    impl Drop for CwdGuard {
        fn drop(&mut self) {
            let _ = env::set_current_dir(&self.0);
        }
    }

    #[test]
    fn test_fs_io() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();

        let file = Path::new("test.txt");
        assert_eq!(write_file(&root, file, "data"), Ok(()));
        assert_eq!(read_file(&root, file), Ok("data".to_string()));

        // Sibling collision
        let sibling = root.parent().unwrap().join(format!("{}-other", root.file_name().unwrap().to_str().unwrap()));
        assert_eq!(write_file(&root, &sibling, "fail"), Err(Error::OutsideBoundary));
    }

    #[test]
    fn test_list_directory() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();

        // Create structure
        fs::create_dir(root.join("subdir")).unwrap();
        fs::write(root.join("b.txt"), "b").unwrap();
        fs::write(root.join("a.txt"), "a").unwrap();
        fs::write(root.join("subdir").join("c.txt"), "c").unwrap();

        // Test root listing
        let entries = list_directory(&root, Path::new(".")).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0], DirEntry { name: "a.txt".to_string(), is_dir: false });
        assert_eq!(entries[1], DirEntry { name: "b.txt".to_string(), is_dir: false });
        assert_eq!(entries[2], DirEntry { name: "subdir".to_string(), is_dir: true });

        // Test subdir listing
        let sub_entries = list_directory(&root, Path::new("subdir")).unwrap();
        assert_eq!(sub_entries.len(), 1);
        assert_eq!(sub_entries[0], DirEntry { name: "c.txt".to_string(), is_dir: false });

        // Test empty directory
        fs::create_dir(root.join("empty")).unwrap();
        let empty_entries = list_directory(&root, Path::new("empty")).unwrap();
        assert!(empty_entries.is_empty());

        // Test path is a file
        assert_eq!(list_directory(&root, Path::new("a.txt")), Err(Error::Io));

        // Test missing directory
        assert_eq!(list_directory(&root, Path::new("missing")), Err(Error::Io));

        // Test boundary violation
        assert_eq!(list_directory(&root, Path::new("..")), Err(Error::OutsideBoundary));
    }

    #[test]
    fn test_discover_files() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();

        // Create structure
        fs::create_dir(root.join("a")).unwrap();
        fs::write(root.join("a/1.txt"), "").unwrap();
        fs::write(root.join("2.txt"), "").unwrap();
        fs::create_dir(root.join("b")).unwrap();

        let files = discover_files(&root).unwrap();
        assert_eq!(files.len(), 4);
        assert_eq!(files[0], PathBuf::from("2.txt"));
        assert_eq!(files[1], PathBuf::from("a"));
        assert_eq!(files[2], PathBuf::from("a/1.txt"));
        assert_eq!(files[3], PathBuf::from("b"));

        // Symlink behavior
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            symlink(root.join("a/1.txt"), root.join("link")).unwrap();
            let files = discover_files(&root).unwrap();
            assert!(files.contains(&PathBuf::from("link")));

            // External symlink
            symlink(dir.path().parent().unwrap().join("other"), root.join("bad_link")).unwrap();
            assert_eq!(discover_files(&root), Err(Error::OutsideBoundary));
        }

        // Windows junction/symlink check would go here if test runner supports it.
    }

    #[test]
    fn test_discover_files_broken_link() {
        // This test verifies that broken links are discovered but cause Error::Io
        // because their safety boundary cannot be definitively proven.

        #[cfg(unix)]
        {
            let dir = tempdir().unwrap();
            let root = dir.path().canonicalize().unwrap();
            use std::os::unix::fs::symlink;

            symlink(root.join("missing"), root.join("broken")).unwrap();
            // validate_boundary fails with Io on broken link
            assert_eq!(discover_files(&root), Err(Error::Io));
        }

        #[cfg(windows)]
        {
            let dir = tempdir().unwrap();
            let root = dir.path().canonicalize().unwrap();
            let missing = root.join("missing");

            // Get any FileType to satisfy the signature
            let existing = root.join("existing");
            fs::write(&existing, "").unwrap();
            let ft = fs::metadata(&existing).unwrap().file_type();

            // Verify that metadata failure in reparse check returns Error::Io
            let result = is_link_or_reparse(&missing, &ft);
            assert_eq!(result, Err(Error::Io));
        }
    }

    #[test]
    fn test_discover_files_windows_junction() {
        #[cfg(windows)]
        {
            let dir = tempdir().unwrap();
            let root = dir.path().canonicalize().unwrap();

            let target = root.join("target");
            fs::create_dir(&target).unwrap();
            fs::write(target.join("file.txt"), "data").unwrap();

            let junction = root.join("link_junction");

            // Use cmd to create junction as it typically doesn't require admin rights
            let output = std::process::Command::new("cmd")
                .args(["/C", "mklink", "/J", junction.to_str().unwrap(), target.to_str().unwrap()])
                .output()
                .unwrap();

            if !output.status.success() {
                // Skip if environment prevents junction creation
                return;
            }

            let files = discover_files(&root).unwrap();

            // Verify junction entry is discovered
            assert!(files.contains(&PathBuf::from("link_junction")));
            // Verify traversal did NOT descend into the junction target
            assert!(!files.contains(&PathBuf::from("link_junction/file.txt")));
            // Verify normal sibling discovery
            assert!(files.contains(&PathBuf::from("target/file.txt")));
        }
    }

    #[test]
    fn test_cwd_independence() {
        let _lock = CWD_MUTEX.lock().unwrap();
        let _guard = CwdGuard::new();

        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let other_dir = tempdir().unwrap();

        env::set_current_dir(other_dir.path()).unwrap();

        let result = write_file(&root, Path::new("cwd_test.txt"), "content");

        assert_eq!(result, Ok(()));
        assert!(root.join("cwd_test.txt").exists());
    }
}
