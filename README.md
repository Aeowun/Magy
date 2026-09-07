# Magy

Magy is a local AI software engineering system built around a simple principle:

> **The AI decides what it wants to do. The runtime decides what it is allowed to do.**

Magy is designed to help a developer give an AI agent a real software project and let it make useful, verifiable progress inside a controlled environment.

The system is built around a Rust core, explicit execution boundaries, replaceable model providers, and controlled tools.

## Philosophy

Magy is not intended to be an AI that is given unrestricted access to a machine and told to "figure it out."

Instead, the system separates **reasoning** from **authority**.

The model can reason about a project and request actions.

The runtime determines whether those actions are valid, permitted, and safe to execute.

```text
User
 ↓
Magy
 ├── Project State
 ├── Agent
 ├── Model
 └── Tools
      ↓
   Runtime
      ↓
   Project
```

This separation is fundamental to the architecture.

## Architecture

Magy is organized around a small layered core:

```text
Application
    ↓
Domain
    ↓
Infrastructure
```

### Domain

The domain defines Magy's rules and core concepts.

It contains the models and state required to represent:

* projects
* project tasks
* agent state
* model interactions
* tool requests and results
* planning and execution concepts

The domain is intended to remain independent of specific UI frameworks, model providers, and operating-system details.

### Application

The application layer coordinates operations across the domain and infrastructure.

It is responsible for turning higher-level intent into controlled execution while preserving domain rules and state transitions.

### Infrastructure

Infrastructure provides the concrete mechanisms required by the system, including filesystem access and communication with model providers.

Infrastructure is where external systems are integrated without allowing them to define Magy's core behavior.

## Agent Model

Magy's agent is modeled as an **authoritative state machine**. The model (AI) participates in reasoning, but neither the model nor the human operator can advance the state directly without runtime validation.

The general lifecycle is:

```text
Idle
  ↓
Planning
  ↓
Executing
  ↓
Verifying
  ↓
Planning
```

The agent also has explicit mechanisms for:

```text
Pause
Resume
Stop
Failure
Completion
```

State transitions are deliberate rather than implicit.

This allows the execution engine to reason about what the agent is currently allowed to do and makes invalid transitions observable instead of silently producing inconsistent state.

## Projects

Magy operates on a user-selected project directory.

The project provides the context in which the agent is allowed to operate.

A project is described by a `Project.md` specification containing information such as:

```text
Project

Goal

Requirements

Constraints

Definition of Done

Tasks

Current Status
```

The project specification acts as a human-readable source of project intent while the Rust domain model provides the structured representation used by the runtime.

## Context and Planning

Before meaningful work can be performed, Magy needs to understand the project it has been given.

The system therefore separates:

```text
Project
   ↓
Project Context
   ↓
Plan
   ↓
Execution
```

Project context represents the relevant project structure and information available to the agent.

Planning converts the project's current state into actionable work while preserving deterministic project semantics.

The model may participate in reasoning, but the runtime remains responsible for enforcing project boundaries and execution rules.

## Dual-Model Protocol

Magy implements a dual-model orchestration protocol that separates high-level project architecture from low-level task execution.

```text
Planner (Architect)  ←→  MAGY Runtime  ←→  Executor (Worker)
      (Qwen)                                   (Nemotron)
```

### Roles and Responsibilities

*   **Planner (Architect)**: Typically a larger model (e.g., Qwen-2.5-7b). It is responsible for analyzing project goals and breaking them down into a structured task list with clear acceptance criteria and required artifacts.
*   **Executor (Worker)**: Typically a smaller, faster model (e.g., Nemotron-3-nano-4b). It is bounded to a single task at a time and cannot modify the project plan or declare the project finished.

Neither model has the authority to advance the project state directly. The **MAGY Runtime** acts as the sole state authority, validating the Planner's output and independently verifying the Executor's evidence before marking a task as complete.

## Models

Magy treats AI models as replaceable roles. The system is optimized for a dual-model setup but can be configured to use a single model for both roles.

### Defaults and Configuration

The CLI and App use the following defaults when connecting to LM Studio:

*   **Planner**: `qwen2.5-7b-instruct`
*   **Executor**: `nvidia/nemotron-3-nano-4b`

To customize the models, set the following environment variables before running the workbench (`Magy.exe`):

```powershell
$env:MAGY_PLANNER_MODEL = "qwen2.5-7b-instruct"
$env:MAGY_EXECUTOR_MODEL = "nvidia/nemotron-3-nano-4b"
./Magy.exe
```

## Tools

Models do not directly manipulate the filesystem.

Instead, the model requests an operation through a controlled tool interface:

```text
Model
  ↓
Tool Request
  ↓
Magy Runtime
  ↓
Tool
  ↓
Tool Result
  ↓
Model
```

Tools provide an explicit authority boundary between what the model **wants** to do and what Magy **allows** it to do.

This is intended to make model actions inspectable, testable, and enforceable.

### Secure execution policy

Tool requests are evaluated by an explicit approval policy before execution.
The default policy:

* approves `ReadFile`, `ListDirectory`, and `DiscoverFiles`;
* marks `WriteFile` as pending for explicit approval; and
* denies `RunCommand`.

Applications that intentionally permit commands can use
`AllowlistApprovalPolicy` and add exact command strings with
`allow_command(...)`. Writes can be explicitly enabled with `allow_writes()`.
The policy-enforced `execute_tool_with_policy` API is available alongside the
legacy execution API for trusted callers.

Model actions are parsed into typed `ModelAction` values from raw JSON, fenced
JSON, or an `{ "action": ... }` envelope. Arbitrary prose is not executable.

## Safety Boundary

A selected project directory is treated as a security boundary.

Operations performed through Magy's filesystem runtime are constrained to that project.

The runtime is responsible for handling issues such as:

* path traversal
* canonicalization
* sibling-prefix collisions
* symbolic links
* junctions and other reparse points
* controlled recursive discovery

Filesystem writes use a temporary file, flush/sync, and atomic replacement.
The default limits are 4 MiB per file, 10,000 entries per directory, and
100,000 discovered entries. File, context, history, command, and cycle limits
can be configured through the corresponding execution APIs; directory and
discovery guards use the published runtime constants.

Higher-level code should use these protected operations rather than bypassing them.

The security model is intentionally explicit about its guarantees and limitations. Magy does not claim that filesystem path validation eliminates every possible race condition between validation and use.

## Bounded execution and audit

Execution contracts support limits for model steps (32 by default), model
history (64 steps), model context (8 MiB), command output (1 MiB), and command
wall-clock time (30 seconds). Command execution remains shell-based and should
only be enabled through an approval policy. A verification policy distinguishes
successful tool results from failures, while `AuditSink` implementations can
record redacted request, approval, completion, and verification events without
persisting tool output.

## Determinism

Deterministic behavior is a core design goal.

The same inputs should produce predictable results wherever practical.

This applies to areas such as:

* project parsing
* project serialization
* agent state transitions
* task identity
* task selection
* project discovery
* planning
* tool dispatch
* persistence

Determinism makes the system easier to test, debug, reproduce, and trust.

## Verification

Magy is designed around the principle that performing an action is not the same thing as proving that the work succeeded.

The intended execution model is:

```text
Execute
   ↓
Verify
   ↓
Complete
```

Verification is therefore treated as part of the execution architecture rather than as an optional reporting step.
Magy includes a generic verification runner that can execute `cargo test`, `npm test`, or custom commands defined in `Project.md`.

## Development

Magy is under active development.

The project is intentionally being built incrementally, with emphasis on:

* explicit architecture
* deterministic behavior
* controlled execution
* strong invariants
* testable boundaries
* replaceable integrations
* small, understandable core components

The architecture and public interfaces may change during development.

Development history and implementation milestones are maintained separately in [`CHANGELOG.md`](CHANGELOG.md).

## Repository

The repository contains the Magy Rust workspace and its core libraries.

The core follows the layered architecture described above rather than coupling the project directly to a desktop UI or a specific model provider.

## Building

Build the project with:

```bash
cargo build
```

Run the test suite with:

```bash
cargo test
```

The CLI uses `nvidia/nemotron-3-nano-4b` by default when connecting to the
local LM Studio provider. To compare another locally loaded model without
changing source code, set `MAGY_MODEL`:

```powershell
$env:MAGY_MODEL = "qwen/qwen3-1.7b"
cargo run -p magy-cli -- C:\path\to\project "node scripts/check.js"
```

Model comparisons should keep the project fixture, verification command,
approval policy, step budget, and model settings constant. A terminal
`Completed` result is not sufficient evidence of correctness; inspect the
generated behavior with an executable acceptance test.

For core-library development:

```bash
cargo test -p magy-core
```

The core implementation remains a library API; policies and limits are
configured by constructing the corresponding Rust types. The optional local
`magy-app` host serves the browser workbench described below when that
application is included in a workspace checkout.

## Local workbench UI

Magy includes a native desktop workbench for managing project execution. When you run `magy-app`, it opens a dedicated engineering interface organized like a compact VS Code workspace:

* **Overview** summarizes the active task, repository, and run state.
* **Activity** shows the real-time agent trace with reasoning, tools, and verification results.
* **Chat** provides a conversational surface for planning and discussing changes.
* **Tasks**, **Source**, and **Settings** expose the project plan, read-only GitHub metadata, and execution preferences.

The workbench allows you to open any local project directory and oversee the Magy runtime as it works through your engineering goals.

## License

Magy is free software licensed under the **GNU General Public License v3.0**.

See [`LICENSE`](LICENSE) for the full license text.
