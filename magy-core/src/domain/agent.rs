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
use tracing::debug;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum State {
    Idle,
    Planning,
    Executing,
    Verifying,
    Paused(Box<State>),
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    Start(PathBuf),
    TaskSelected(Task),
    ActionDone,
    TestsPassed,
    TestsFailed,
    PlanCorrection,
    AllDone,
    Pause,
    Resume,
    Stop,
    FatalError,
}

#[derive(Debug, PartialEq)]
pub struct Agent {
    state: State,
    root: Option<PathBuf>,
    task: Option<Task>,
}

impl Default for Agent {
    fn default() -> Self {
        Self {
            state: State::Idle,
            root: None,
            task: None,
        }
    }
}

impl Agent {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    pub fn task(&self) -> Option<&Task> {
        self.task.as_ref()
    }

    pub fn transition(&mut self, event: Event) -> Result<(), Error> {
        let mut next_root = self.root.clone();
        let mut next_task = self.task.clone();

        let next_state = match (&self.state, &event) {
            // Idle transitions
            (State::Idle, Event::Start(path)) => {
                next_root = Some(path.clone());
                State::Planning
            },

            // Planning transitions
            (State::Planning, Event::TaskSelected(task)) => {
                next_task = Some(task.clone());
                State::Executing
            },
            (State::Planning, Event::Pause) => State::Paused(Box::new(State::Planning)),

            // Executing transitions
            (State::Executing, Event::ActionDone) => State::Verifying,
            (State::Executing, Event::Pause) => State::Paused(Box::new(State::Executing)),

            // Verifying transitions
            (State::Verifying, Event::TestsFailed) => State::Executing,
            (State::Verifying, Event::PlanCorrection) => State::Planning,
            (State::Verifying, Event::TestsPassed) => {
                next_task = None;
                State::Planning
            },
            (State::Verifying, Event::AllDone) => State::Completed,
            (State::Verifying, Event::Pause) => State::Paused(Box::new(State::Verifying)),

            // Paused transitions
            (State::Paused(prev), Event::Resume) => *prev.clone(),
            (State::Paused(_), Event::FatalError) => State::Failed,

            // Global Stop (Any non-Idle state to Idle)
            (s, Event::Stop) if s != &State::Idle => {
                next_root = None;
                next_task = None;
                State::Idle
            },

            // Global Fatal Error (Active states to Failed)
            (State::Planning, Event::FatalError) => State::Failed,
            (State::Executing, Event::FatalError) => State::Failed,
            (State::Verifying, Event::FatalError) => State::Failed,

            // Otherwise, invalid
            (s, e) => {
                debug!(current_state = ?s, event = ?e, "Invalid state transition requested");
                return Err(Error::InvalidStateTransition);
            }
        };

        self.state = next_state;
        self.root = next_root;
        self.task = next_task;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_task() -> Task {
        Task {
            id: "1".to_string(),
            description: "Test task".to_string(),
        }
    }

    #[test]
    fn test_lifecycle_flow() {
        let mut agent = Agent::new();
        assert_eq!(agent.state(), &State::Idle);

        agent.transition(Event::Start(PathBuf::from("/test"))).unwrap();
        assert_eq!(agent.state(), &State::Planning);

        agent.transition(Event::TaskSelected(sample_task())).unwrap();
        assert_eq!(agent.state(), &State::Executing);

        agent.transition(Event::ActionDone).unwrap();
        assert_eq!(agent.state(), &State::Verifying);

        agent.transition(Event::AllDone).unwrap();
        assert_eq!(agent.state(), &State::Completed);
    }

    #[test]
    fn test_pause_resume() {
        let mut agent = Agent::new();
        agent.transition(Event::Start(PathBuf::from("/test"))).unwrap();
        agent.transition(Event::TaskSelected(sample_task())).unwrap();
        assert_eq!(agent.state(), &State::Executing);

        agent.transition(Event::Pause).unwrap();
        match agent.state() {
            State::Paused(prev) => assert_eq!(**prev, State::Executing),
            _ => panic!("Expected Paused state"),
        }

        agent.transition(Event::Resume).unwrap();
        assert_eq!(agent.state(), &State::Executing);
    }

    #[test]
    fn test_stop_reset() {
        let mut agent = Agent::new();

        // Stop from Planning
        agent.transition(Event::Start(PathBuf::from("/test"))).unwrap();
        agent.transition(Event::Stop).unwrap();
        assert_eq!(agent.state(), &State::Idle);

        // Stop from Failed
        agent.transition(Event::Start(PathBuf::from("/test"))).unwrap();
        agent.transition(Event::FatalError).unwrap();
        assert_eq!(agent.state(), &State::Failed);
        agent.transition(Event::Stop).unwrap();
        assert_eq!(agent.state(), &State::Idle);

        // Stop from Completed
        agent.transition(Event::Start(PathBuf::from("/test"))).unwrap();
        agent.transition(Event::TaskSelected(sample_task())).unwrap();
        agent.transition(Event::ActionDone).unwrap();
        agent.transition(Event::AllDone).unwrap();
        assert_eq!(agent.state(), &State::Completed);
        agent.transition(Event::Stop).unwrap();
        assert_eq!(agent.state(), &State::Idle);
    }

    #[test]
    fn test_invalid_transition() {
        let mut agent = Agent::new();

        // Idle -> ActionDone is invalid
        let res = agent.transition(Event::ActionDone);
        assert_eq!(res, Err(Error::InvalidStateTransition));
        assert_eq!(agent.state(), &State::Idle);
    }

    #[test]
    fn test_fatal_error() {
        let mut agent = Agent::new();
        agent.transition(Event::Start(PathBuf::from("/test"))).unwrap();
        agent.transition(Event::TaskSelected(sample_task())).unwrap();
        agent.transition(Event::ActionDone).unwrap();
        assert_eq!(agent.state(), &State::Verifying);

        agent.transition(Event::FatalError).unwrap();
        assert_eq!(agent.state(), &State::Failed);
    }

    #[test]
    fn test_root_initialization() {
        let mut agent = Agent::new();
        assert_eq!(agent.root(), None);

        let root_path = PathBuf::from("/project");
        agent.transition(Event::Start(root_path.clone())).unwrap();
        assert_eq!(agent.root(), Some(root_path.as_path()));
    }

    #[test]
    fn test_root_persistence() {
        let mut agent = Agent::new();
        let root_path = PathBuf::from("/project");
        agent.transition(Event::Start(root_path.clone())).unwrap();

        agent.transition(Event::TaskSelected(sample_task())).unwrap();
        assert_eq!(agent.state(), &State::Executing);
        assert_eq!(agent.root(), Some(root_path.as_path()));
    }

    #[test]
    fn test_root_cleared_on_stop() {
        let mut agent = Agent::new();
        let root_path = PathBuf::from("/project");
        agent.transition(Event::Start(root_path)).unwrap();

        agent.transition(Event::Stop).unwrap();
        assert_eq!(agent.state(), &State::Idle);
        assert_eq!(agent.root(), None);
    }

    #[test]
    fn test_task_selection() {
        let mut agent = Agent::new();
        agent.transition(Event::Start(PathBuf::from("/test"))).unwrap();

        let task = sample_task();
        agent.transition(Event::TaskSelected(task.clone())).unwrap();

        assert_eq!(agent.state(), &State::Executing);
        assert_eq!(agent.task(), Some(&task));
    }

    #[test]
    fn test_task_cleared_on_success() {
        let mut agent = Agent::new();
        agent.transition(Event::Start(PathBuf::from("/test"))).unwrap();
        agent.transition(Event::TaskSelected(sample_task())).unwrap();
        agent.transition(Event::ActionDone).unwrap();
        assert_eq!(agent.state(), &State::Verifying);

        agent.transition(Event::TestsPassed).unwrap();
        assert_eq!(agent.state(), &State::Planning);
        assert_eq!(agent.task(), None);
    }

    #[test]
    fn test_task_persistence_on_failure() {
        let mut agent = Agent::new();
        agent.transition(Event::Start(PathBuf::from("/test"))).unwrap();
        let task = sample_task();
        agent.transition(Event::TaskSelected(task.clone())).unwrap();
        agent.transition(Event::ActionDone).unwrap();

        agent.transition(Event::TestsFailed).unwrap();
        assert_eq!(agent.state(), &State::Executing);
        assert_eq!(agent.task(), Some(&task));
    }

    #[test]
    fn test_task_cleared_on_stop() {
        let mut agent = Agent::new();
        agent.transition(Event::Start(PathBuf::from("/test"))).unwrap();
        agent.transition(Event::TaskSelected(sample_task())).unwrap();

        agent.transition(Event::Stop).unwrap();
        assert_eq!(agent.state(), &State::Idle);
        assert_eq!(agent.root(), None);
        assert_eq!(agent.task(), None);
    }
}
