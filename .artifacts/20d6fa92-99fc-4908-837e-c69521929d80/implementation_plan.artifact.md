# Implementation Plan - Dual-Model Protocol (Qwen Planner + Nemotron Executor)

This plan implements a strict separation of concerns between a **Planner** (Qwen) and an **Executor** (Nemotron) to prevent overthinking and maintain global project control.

## User Review Required

> [!IMPORTANT]
> **Role Split Architecture**: This change fundamentally alters how `magy-core` processes tasks. The Planner is the only one allowed to modify the task list; the Executor is strictly bounded to one task at a time.
> **Project.md Authority**: `Project.md` will become a human-readable projection of the internal state, rather than being parsed as the primary state on every step.

## Proposed Changes

### Domain Layer
Define the new protocol contracts to distinguish between planning and execution.

#### [MODIFY] [model.rs](file:///C:/Dev/Projects/Magy/magy-core/src/domain/model.rs)
- Add `PlannerRequest` and `PlannerResponse` (structured task list).
- Add `ExecutorRequest` and `ExecutorResponse` (bounded tool calls).
- Refine `RunState` to include explicit `AwaitingPlan` and `ExecutingTask` phases.

#### [MODIFY] [project.rs](file:///C:/Dev/Projects/Magy/magy-core/src/domain/project.rs)
- Add `AcceptanceCriteria` to `ProjectTask`.
- Add `Evidence` field to `ProjectTask` to store verification artifacts.

---

### Application Layer
Update the workflow to orchestrate the two models.

#### [MODIFY] [planning.rs](file:///C:/Dev/Projects/Magy/magy-core/src/application/planning.rs)
- Transform from deterministic task filter to an LLM-backed `plan_project` function.
- Implement validation for Qwen's output (id consistency, dependency loops).

#### [MODIFY] [execution.rs](file:///C:/Dev/Projects/Magy/magy-core/src/application/execution.rs)
- Constrain `run_execution_cycle` to receive only the *active* task and its scope.
- Enforce that Nemotron cannot call `task_complete` without providing evidence or meeting criteria.

#### [MODIFY] [coordinator.rs](file:///C:/Dev/Projects/Magy/magy-core/src/application/coordinator.rs)
- Update `run_project_workflow` to use the dual-model split.
- Step 1: Call Qwen to initialize/update the plan.
- Step 2: Loop through tasks, calling Nemotron for each.

---

### Infrastructure Layer
Provide the actual implementations for the model roles.

#### [MODIFY] [model.rs](file:///C:/Dev/Projects/Magy/magy-core/src/infrastructure/model.rs)
- Implement `PlannerProvider` (Qwen optimized).
- Implement `ExecutorProvider` (Nemotron optimized).
- Update prompt templates to reflect the new role constraints.

## Verification Plan

### Automated Tests
- `cargo test --package magy-core`: Verify new state transitions and protocol validation.
- Unit tests for `plan_project` validation logic (detecting invalid task graphs).

### Manual Verification
- Run Magy against a complex multi-step prompt (e.g., "Implement a Todo app with Persistence and UI").
- Observe `Project.md` updates to ensure it reflects the Planner's breakdown correctly.
- Verify Nemotron does not attempt to create new tasks during execution.
