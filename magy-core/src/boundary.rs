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

use crate::Error;
use std::path::{Component, Path, PathBuf};
use tracing::debug;

/// Security boundary for Magy. Ensures target paths are within the project root.
pub fn is_within_boundary(root: &Path, target: &Path) -> bool {
    // 1. Canonicalize root to get the stable, absolute identity.
    let root_c = match root.canonicalize() {
        Ok(p) => p,
        Err(_) => return false,
    };

    validate_boundary(&root_c, target).is_ok()
}

/// A more granular boundary validator that returns specific Errors.
/// root_c MUST be a canonicalized absolute path.
pub fn validate_boundary(root_c: &Path, target: &Path) -> Result<(), Error> {
    // 1. Resolve target against root (if relative) and normalize textually to resolve '..'.
    let mut absolute_target = if target.is_absolute() {
        target.to_path_buf()
    } else {
        root_c.join(target)
    };
    absolute_target = normalize_path(&absolute_target);

    // 2. Walk up to the nearest existing entry.
    // We use symlink_metadata to avoid skipping links (even broken ones).
    let mut current = absolute_target.as_path();
    loop {
        if let Ok(md) = current.symlink_metadata() {
            // Found the nearest existing filesystem entry.
            // If it's a link (symlink or Windows reparse point), it MUST be resolvable
            // and point inside the root. Broken links result in Error::Io.
            let is_link = md.file_type().is_symlink() || is_reparse_point_md(&md);

            let base_c = current.canonicalize().map_err(|_| Error::Io)?;
            if !base_c.starts_with(root_c) {
                debug!(
                    root = ?root_c,
                    target = ?absolute_target,
                    base = ?base_c,
                    is_link = is_link,
                    "Boundary violation"
                );
                return Err(Error::OutsideBoundary);
            }
            return Ok(());
        }

        if let Some(parent) = current.parent() {
            current = parent;
        } else {
            break;
        }
    }

    // Should be unreachable if root_c exists
    Err(Error::Io)
}

#[cfg(windows)]
fn is_reparse_point_md(md: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    (md.file_attributes() & 0x400) != 0
}

#[cfg(not(windows))]
fn is_reparse_point_md(_md: &std::fs::Metadata) -> bool {
    false
}

/// A path normalization implementation that resolves '..' and '.' without disk access.
fn normalize_path(path: &Path) -> PathBuf {
    let mut comps = Vec::new();
    let mut prefix = None;
    let mut has_root = false;

    for comp in path.components() {
        match comp {
            Component::Prefix(p) => prefix = Some(p),
            Component::RootDir => has_root = true,
            Component::ParentDir => {
                comps.pop();
            }
            Component::CurDir => {}
            Component::Normal(c) => comps.push(c),
        }
    }

    let mut res = PathBuf::new();
    if let Some(p) = prefix {
        res.push(p.as_os_str());
    }
    if has_root {
        res.push(std::path::MAIN_SEPARATOR.to_string());
    }
    for c in comps {
        res.push(c);
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_security_boundary() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();

        assert!(is_within_boundary(&root, Path::new("test.txt")));
        assert!(!is_within_boundary(&root, Path::new("../escape.txt")));
        assert!(!is_within_boundary(&root, Path::new("C:/Windows/System32")));

        // Sibling collision check: /project-other should NOT start with /project
        let sibling = root.parent().unwrap().join(format!(
            "{}-other",
            root.file_name().unwrap().to_str().unwrap()
        ));
        assert!(!is_within_boundary(&root, &sibling));

        // Windows casing check
        let root_str = root.to_str().unwrap();
        let root_mixed = PathBuf::from(if cfg!(windows) {
            root_str.to_uppercase()
        } else {
            root_str.to_string()
        });
        assert!(is_within_boundary(&root, &root_mixed.join("file.txt")));
    }
}
