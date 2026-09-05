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

Magy's agent is modeled as an explicit state machine.

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

## Models

Magy treats AI models as replaceable components.

The core communicates with models through a provider abstraction rather than embedding one specific model implementation into the agent itself.

Conceptually:

```text
Agent
   ↓
Model Provider
   ↓
Local Model Runtime
```

This allows different local model backends to be introduced without redesigning the agent architecture.

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

Higher-level code should use these protected operations rather than bypassing them.

The security model is intentionally explicit about its guarantees and limitations. Magy does not claim that filesystem path validation eliminates every possible race condition between validation and use.

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

For core-library development:

```bash
cargo test -p magy-core
```

## License

Magy is free software licensed under the **GNU General Public License v3.0**.

See [`LICENSE`](LICENSE) for the full license text.
