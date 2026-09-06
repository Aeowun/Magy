# Changelog

## Unreleased

### Secure local runtime

* Merged the secure execution behavior into the current CLI/UI architecture.
* Added deny-by-default command approval with exact command allowlisting.
* Added bounded file reads/writes and synchronized temporary-file replacement.
* Added command timeouts and bounded stdout/stderr capture.
* Added Windows launch and developer-shell scripts.
* Added explicit model instructions against invented Git/repository setup and
  unrelated shell commands.
* Denied actions now remain visible and trigger a replacement-action attempt
  instead of silently stopping the workflow.
* Added the UI auto-execute toggle for safe file tools.
* Added project-aware verification selection for Rust, Node, Python, and static
  projects; the CLI now defaults to `auto` instead of blindly running Cargo.
* Added explicit `Verification` acceptance contracts in `Project.md`, with
  project-defined commands taking precedence over auto-detected runners.
* Prevented generic/static validation from claiming task acceptance when no
  verifier exists.
* Failed verification is now presented as an amber warning with output, while
  genuine runtime and tool failures remain errors.
* Added local conversational chat through the UI without starting an agent
  execution cycle.
* Fixed approval resumption to restore the active task before executing the
  approved request, preventing approved writes from being discarded because
  the reconstructed agent was still in planning state.
* Added retry handling for malformed structured model actions and regression
  coverage for invalid flat-schema requests.
* Made UI asset serving independent of the process launch directory.
* Added lifecycle hardening: overlapping runs are rejected, worker/project
  failures are surfaced, missing project state no longer panics approval
  resolution, and empty commands are denied even if accidentally allowlisted.

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
* Added distinct `FileNotFound` handling for missing files.

### Project Context

* Added `ProjectContext`, `FileContext`, and `FileContent`.
* Added deterministic project context assembly from `Project.md` and discovered project files.
* Added handling for directories, readable text files, and unreadable/special files.

### Project Initialization

* Added `initialize_project` for creating a `Project.md` from a user-supplied goal.
* Added structured LLM project generation using a dedicated JSON Schema.
* Added conversion from generated project data into the existing `Project` domain model.
* Added protection against overwriting an existing `Project.md`.
* Added failure handling that prevents partial project creation when model output is invalid.
* Verified project generation against a real local Nemotron model through LM Studio.
* Verified generated project data can be serialized to `Project.md` and parsed back into the domain model.

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

* Added **Project-Scoped Command Execution** tool (`RunCommand`).

  * Commands are executed via the local shell but strictly constrained to the project root as the working directory.
  * Captures stdout, stderr, and exit codes for feeding back into the model context.

* Enforced Agent execution-state checks before tool execution.

* Kept all tool access behind the existing project filesystem boundary.

### Model Integration

* Added the `ModelProvider` abstraction.
* Added `ModelRequest` and `ModelResponse`.
* Added optional request-specific JSON Schema support through `ModelRequest`.
* Added LM Studio provider integration through the OpenAI-compatible local API.
* Added configurable LM Studio base URL and model name.
* Added deterministic provider error handling.
* Added mocked provider verification.
* Added provider tests for custom structured-output schemas.
* Updated prompt generation to include execution history, enabling multi-step autonomous reasoning.

### Structured Output

* Replaced branching `oneOf` tool schemas with a flat JSON Schema using an enum discriminator.
* Required `tool`, `path`, `content`, and `command` fields in tool requests, using `null` for fields that do not apply.
* Verified the flat schema against Gemma and Nemotron through LM Studio.
* Added a regression test covering the flat-schema execution cycle.
* The flat schema is now the standard tool-output format used by Magy.

### Agent Reasoning

* Added `ModelAction` for representing model-requested tool actions.
* Added `StepResult` to represent a single reasoning/execution interaction.
* Added bounded reasoning-step orchestration connecting the model provider to the controlled tool runtime.
* Added model-response tool-call parsing.
* Added tool-result propagation into the reasoning result.
* Verified model/tool interaction with mocked providers.
* Verified boundary violations remain enforced when actions originate from model responses.

### Agent Execution

* Added the **bounded Agent Execution Cycle** (`run_execution_cycle`).
* Added termination conditions for the execution loop (max steps reached, tool failure, no action, approval required).
* Added `ExecutionTrace` to preserve a complete record of proposals, decisions, and outcomes during a cycle.
* Added a regression test covering flat-schema tool selection through execution completion.

### Action Verification & Approval

* Added a deterministic **Approval Policy** boundary between reasoning and execution.
* Added `ActionRecord` to represent an audited action with its approval status and outcome.
* Added `ApprovalStatus` and `ExecutionOutcome` domain types.
* Implemented `DefaultApprovalPolicy` that automatically approves read-only actions and holds destructive or system-level actions (write, command) for approval.
* Added **Operator Resolution** API (`resolve_pending_action`) allowing humans to explicitly approve or deny pending actions, which then execute the exact recorded request.

### Application Layer

* Decomposed the application layer into focused responsibility-driven modules:

  * `project_lifecycle`
  * `task_lifecycle`
  * `context_assembly`
  * `planning`
  * `reasoning`
  * `approval`
  * `tool_execution`
  * `execution`
  * `verification_runner`
  * `coordinator`

* Enforced a mandatory **300-line file limit** for all application source files to ensure modularity.

* Established **Agent Run Coordinator** (`run_project_workflow`) as the single entry point for driving the autonomous loop from initialization to project completion.

### Autonomous Task Verification

* Added **Automated Verification Loop** connecting reasoning to the Agent state machine.
* Added `ToolRequest::TaskComplete` virtual action for model signaling.
* Implemented **Project-Anchored Verification Runner** that executes a caller-supplied command (e.g., `cargo test`) to determine success.
* Implemented **Self-Correction Feedback Loop**: failed verifications transition the Agent back to `Executing` and inject structured failure logs into the next model reasoning cycle.
* Enforced deterministic **Bounded Verification Retries** to prevent infinite fix-verify loops.
* Verified the complete write → approval → verification → task completion workflow end to end.

### Interaction Layer

* Added the **Magy CLI** (`magy-cli`) binary crate for command-line operation.
* Implemented real-time execution display including model reasoning and tool output in the CLI.
* Implemented interactive **Operator Decision Loop** for pending actions.
* Created the **Magy UI** (`magy-ui`) using HTML, CSS, and plain JavaScript.
* Established the **Magy App** (`magy-app`) backend using Axum to host the UI and bridge it to the Magy runtime.
* Implemented **Real-time Event Streaming** using Server-Sent Events (SSE) to update the UI as the agent reasons and executes tools.
* Integrated **Native Directory Selection** using the `rfd` crate, allowing users to pick project folders through standard OS dialogs.
* Connected the UI to the complete Magy workflow: project initialization, task planning, interactive approval, and automated verification.
* Verified the end-to-end loop: Load Project → Enter Goal → Generate Project.md → Approve Action → Verify → Task Complete.

### Infrastructure

* Added project-root anchored process execution.
* Renamed modules for better responsibility communication (`fs` -> `filesystem`, `cmd` -> `command`).

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
* Verified safe multi-step command sequences.
* Verified the approval/verification boundary for both permitted and restricted tools.
* Verified that restricted tools (like `RunCommand`) are correctly held in the `AwaitingApproval` state.
* Verified the **Operator Approval → Resume** workflow.
* Verified the **Fail → Fix → Pass** autonomous verification loop.
* Verified LM Studio provider communication and provider failures.
* Verified custom structured-output schema transmission.
* Verified project initialization from a real local model.
* Verified bounded model-to-tool reasoning.
* Core test suite currently passes **81 tests** with **0 failures**.

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

* Real-time graphical user approval interface.
* Specialized File Patch/Edit tool (for sub-file modifications).
* Multi-model consensus or verification agents.
* Git automation and commit history integration.
* Persistent Agent runs (database-backed).
* Multi-agent orchestration.
* Cloud provider support.

## Release Policy

Magy uses semantic versioning for releases.

Development tasks and architectural milestones are tracked under `Unreleased` and do not independently increment the project version.

Version changes represent actual release boundaries rather than individual implementation tasks.
