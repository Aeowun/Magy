# Changelog

All notable changes to Magy are documented here.

> [!TIP]
> **Unreleased** — Magy is currently in active core development. The implementation is being built incrementally, with each milestone verified before new capabilities are added.

### Core Foundation

* Established the Rust workspace and `magy-core` library.
* Established separation between application, domain, and infrastructure layers.
* Added project-scoped filesystem boundary enforcement.
* Added safe filesystem read/write primitives.
* Added structured tracing with `tracing`.

### Agent Domain

* Added deterministic Agent lifecycle state machine.
* Added project-root binding to Agent runs.
* Added active task identity.
* Added pause/resume semantics.
* Added deterministic stop and failure handling.
* Added task selection orchestration.
* Added task completion orchestration with persistence-aware state coordination.

### Project Model

* Added the `Project` domain model covering:

  * project name
  * goal
  * requirements
  * constraints
  * definition of done
  * tasks
  * current status

* Added `ProjectTask` and `TaskStatus`.

* Added specification-driven `Project.md` parsing.

* Added canonical `Project.md` serialization.

* Verified parser/serializer round-trip integrity.

### Filesystem / Discovery

* Added deterministic non-recursive directory listing.
* Added recursive project file discovery.
* Added Windows junction/reparse-point protection.
* Verified symlink/junction traversal does not escape or recurse through reparse points.
* Verified CWD-independent filesystem behavior.

### Project Context

* Added `ProjectContext`, `FileContext`, and `FileContent`.
* Added deterministic project context assembly from `Project.md` and discovered project files.
* Added handling for directories, readable text files, and unreadable/special files.

### Agent Planning

* Added `ProjectPlan`.
* Added deterministic planning from open project tasks.
* Completed tasks are excluded from execution plans.

### Tool Execution

* Added `ToolRequest` and `ToolResult`.

* Added the Rust-side tool execution surface for:

  * read file
  * write file
  * list directory
  * discover files

* Enforced Agent execution-state checks before tool execution.

* Kept all tool access behind the existing project filesystem boundary.

### Model Integration

* Added the `ModelProvider` abstraction.
* Added `ModelRequest` and `ModelResponse`.
* Added LM Studio provider integration through the OpenAI-compatible local API.
* Added configurable LM Studio base URL and model name.
* Added deterministic provider error handling.
* Added mocked provider verification.

### Agent Reasoning

* Added `ModelAction` for representing model-requested tool actions.
* Added bounded reasoning-step orchestration connecting the model provider to the controlled tool runtime.
* Added model-response tool-call parsing.
* Added tool-result propagation into the reasoning result.
* Verified model/tool interaction with mocked providers.
* Verified boundary violations remain enforced when actions originate from model responses.

### Application Layer

* Added project opening orchestration:

  * read `Project.md`
  * parse project state
  * initialize Agent

* Added project persistence orchestration.

* Added task selection orchestration.

* Added task completion orchestration using shadow project updates.

* Added project context assembly.

* Added deterministic execution planning.

* Added controlled tool execution.

* Added bounded reasoning-step orchestration.

### Verification

* Verified filesystem traversal protection.
* Verified recursive project discovery.
* Verified Windows junction/reparse-point handling.
* Verified sibling-boundary collision protection.
* Verified relative paths remain anchored to the project root regardless of process CWD.
* Verified parser handling of required project sections.
* Verified canonical project serialization.
* Verified Agent lifecycle transitions and invalid-transition behavior.
* Verified task selection and task completion failure semantics.
* Verified persistence failure does not mutate the in-memory Project or advance the Agent.
* Verified project context assembly.
* Verified deterministic open-task planning.
* Verified controlled tool execution.
* Verified LM Studio provider communication and provider failures.
* Verified bounded model-to-tool reasoning.
* Core test suite currently passes **58 tests**.

## [0.1.0]

Initial development release of the Magy core foundation.

This release establishes the deterministic, project-scoped foundation required for future AI-assisted execution.

### Included

* Rust `magy-core` library.
* Secure project filesystem boundary.
* Safe file I/O.
* Deterministic Agent state machine.
* Project domain model.
* `Project.md` parser and serializer.
* Project open/save orchestration.
* Task selection and completion orchestration.

### Not Yet Included

* AI provider integration.
* Autonomous Agent loop.
* Tool registry or tool-calling interface.
* Command execution.
* Test-runner integration.
* Live activity/event system.
* Git automation.
* Persistent Agent runs.
* Desktop UI.
* Multi-agent functionality.
* Cloud services.

## Release Policy

Magy uses semantic versioning for releases.

Development tasks and architectural milestones are tracked under `Unreleased` and do not independently increment the project version.

Version changes represent actual release boundaries rather than individual implementation tasks.
