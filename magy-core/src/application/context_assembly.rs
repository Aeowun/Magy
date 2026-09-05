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

use crate::domain::project::{FileContent, FileContext, Project, ProjectContext};
use crate::infrastructure::filesystem::{discover_files, read_file};
use crate::Error;
use std::path::PathBuf;

/// Assembles the complete project context: model and file tree with content.
pub fn assemble_project_context(root: PathBuf, project: Project) -> Result<ProjectContext, Error> {
    let paths = discover_files(&root)?;
    let mut files = Vec::new();

    for rel_path in paths {
        let abs_path = root.join(&rel_path);
        let md = std::fs::symlink_metadata(&abs_path).map_err(|_| Error::Io)?;

        let content = if md.is_dir() {
            FileContent::Directory
        } else if md.is_file() {
            match read_file(&root, &rel_path) {
                Ok(text) => FileContent::Text(text),
                Err(_) => FileContent::Unreadable("Read failed".to_string()),
            }
        } else {
            FileContent::Unreadable("Special file".to_string())
        };

        files.push(FileContext {
            path: rel_path,
            content,
        });
    }

    Ok(ProjectContext { project, files })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::project_lifecycle::open_project;
    use std::fs;
    use std::path::{Path, PathBuf};
    use tempfile::tempdir;

    #[test]
    fn test_assemble_project_context() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let content = "My Project\n\nGoal\nTest\n\nTasks\n- [ ] T1";
        fs::write(root.join("Project.md"), content).unwrap();

        fs::create_dir(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "fn main() {}").unwrap();
        fs::create_dir(root.join("empty")).unwrap();

        let (_, project) = open_project(root.clone()).unwrap();
        let context = assemble_project_context(root, project.clone()).unwrap();

        assert_eq!(context.project, project);
        assert_eq!(context.files.len(), 4);

        let p_md = &context.files[0];
        assert_eq!(p_md.path, PathBuf::from("Project.md"));
        assert!(matches!(p_md.content, FileContent::Text(_)));
    }

    #[test]
    fn test_assemble_project_context_unreadable() {
        let dir = tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::write(root.join("Project.md"), "P\n\nGoal\nG\n\nTasks\n- [ ] T").unwrap();
        fs::write(root.join("binary.bin"), vec![0, 159, 146, 150]).unwrap();

        let (_, project) = open_project(root.clone()).unwrap();
        let context = assemble_project_context(root, project).unwrap();

        let bin_entry = context
            .files
            .iter()
            .find(|f| f.path == Path::new("binary.bin"))
            .unwrap();
        assert!(matches!(bin_entry.content, FileContent::Unreadable(_)));
    }
}
