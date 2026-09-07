// Copyright (C) 2026 Zachary Joubert
//
// This file is part of Magy.

use crate::domain::agent::Agent;
use crate::domain::model::{ExecutionTrace, FailureReason};
use crate::domain::project::Project;
use crate::Error;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const SNAPSHOT_DIRECTORY: &str = ".magy";
pub const SNAPSHOT_FILE: &str = "run-snapshot.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunSnapshot {
    pub version: u8,
    pub sequence: u64,
    pub updated_at_ms: u64,
    pub worker_active: bool,
    pub trace: ExecutionTrace,
    pub project: Project,
    pub agent: Option<Agent>,
}

impl RunSnapshot {
    pub fn new(
        sequence: u64,
        worker_active: bool,
        trace: ExecutionTrace,
        project: Project,
        agent: Option<Agent>,
    ) -> Self {
        Self {
            version: 1,
            sequence,
            updated_at_ms: now_ms(),
            worker_active,
            trace,
            project,
            agent,
        }
    }
}

pub fn snapshot_path(root: &Path) -> PathBuf {
    root.join(SNAPSHOT_DIRECTORY).join(SNAPSHOT_FILE)
}

/// Persist a run snapshot atomically, without changing Project.md.
pub fn save_run_snapshot(root: &Path, snapshot: &RunSnapshot) -> Result<(), Error> {
    let directory = root.join(SNAPSHOT_DIRECTORY);
    fs::create_dir_all(&directory).map_err(|_| Error::Io)?;
    let path = snapshot_path(root);
    let temporary = directory.join(format!("{}.tmp", SNAPSHOT_FILE));
    let content = serde_json::to_vec_pretty(snapshot).map_err(|_| Error::Io)?;
    fs::write(&temporary, content).map_err(|_| Error::Io)?;
    if fs::rename(&temporary, &path).is_err() {
        // Windows cannot rename over an existing destination. The temporary
        // file still prevents readers from observing a partial JSON document.
        #[cfg(windows)]
        {
            let _ = fs::remove_file(&path);
            if fs::rename(&temporary, &path).is_ok() {
                return Ok(());
            }
        }
        let _ = fs::remove_file(&temporary);
        Err(Error::Io)
    } else {
        Ok(())
    }
}

pub fn load_run_snapshot(root: &Path) -> Result<Option<RunSnapshot>, Error> {
    let path = snapshot_path(root);
    match fs::read_to_string(path) {
        Ok(content) => serde_json::from_str(&content)
            .map(Some)
            .map_err(|_| Error::Io),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(Error::Io),
    }
}

/// Converts an active snapshot left by a crashed worker into one terminal
/// failure. This prevents a restarted UI from displaying a permanently
/// running operation.
pub fn recover_orphaned_snapshot(snapshot: &mut RunSnapshot, stale_after_ms: u64) -> bool {
    if !snapshot.worker_active
        || !snapshot.trace.run.state().is_active()
        || now_ms().saturating_sub(snapshot.updated_at_ms) < stale_after_ms
    {
        return false;
    }

    snapshot.worker_active = false;
    snapshot.trace.run.fail(FailureReason::Interrupted(
        "Recovered orphaned worker".to_string(),
    ));
    snapshot.updated_at_ms = now_ms();
    true
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::model::RunState;
    use crate::domain::project::{Project, ProjectTask, TaskStatus};
    use tempfile::tempdir;

    fn project() -> Project {
        Project {
            name: "P".to_string(),
            goal: "G".to_string(),
            requirements: vec![],
            constraints: vec![],
            definition_of_done: vec![],
            tasks: vec![ProjectTask {
                id: "1".to_string(),
                description: "T".to_string(),
                status: TaskStatus::Open,
                acceptance_criteria: vec![],
                evidence: vec![],
            }],
            current_status: "Running".to_string(),
            plan_version: 0,
            plan_created_at_ms: None,
            replan_count: 0,
            replan_reason: None,
        }
    }

    #[test]
    fn snapshot_round_trips_atomically() {
        let dir = tempdir().unwrap();
        let snapshot = RunSnapshot::new(4, true, ExecutionTrace::new(), project(), None);
        save_run_snapshot(dir.path(), &snapshot).unwrap();
        assert_eq!(load_run_snapshot(dir.path()).unwrap(), Some(snapshot));
        let replacement = RunSnapshot::new(5, false, ExecutionTrace::new(), project(), None);
        save_run_snapshot(dir.path(), &replacement).unwrap();
        assert_eq!(load_run_snapshot(dir.path()).unwrap(), Some(replacement));
        assert!(!dir
            .path()
            .join(SNAPSHOT_DIRECTORY)
            .join("run-snapshot.json.tmp")
            .exists());
    }

    #[test]
    fn stale_active_snapshot_is_recovered_once() {
        let mut snapshot = RunSnapshot::new(1, true, ExecutionTrace::new(), project(), None);
        snapshot.trace.start(1);
        snapshot.trace.run.begin_planning();
        snapshot.updated_at_ms = 0;
        assert!(recover_orphaned_snapshot(&mut snapshot, 1));
        assert_eq!(snapshot.trace.run.state(), &RunState::Failed);
        assert!(!snapshot.worker_active);
        assert!(!recover_orphaned_snapshot(&mut snapshot, 1));
        assert_eq!(snapshot.trace.run.terminal_event_count(), 1);
    }
}
