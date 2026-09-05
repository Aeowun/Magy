# Magy

Magy is a local AI software engineering system.

The model proposes actions. The runtime validates and executes them according to the system's rules.

Magy consists of a Rust core, model-provider interfaces, controlled tools, project state, and a graphical user interface.

## Architecture

Magy separates model reasoning from action execution.

```text
User
 ↓
Magy App (UI)
 ↓
Coordinator
 ├── Project State
 ├── Agent
 ├── Model
 └── Tools
      ↓
   Runtime
      ↓
   Project
```

The model does not directly modify the project. It produces requests that are interpreted and executed by the runtime.

## Usage

### Requirements

Magy expects a local model server running at:

```text
http://localhost:1234/v1
```

LM Studio works by default.

Load a chat/instruct model before starting Magy.

### Run Magy

Start the Magy application:

```text
cargo run -p magy-app
```

Then open your browser to `http://localhost:3000`.

### To load a project

1.  Click **Load Project**.
2.  Select a project directory via the native folder picker. Magy will set this as the project root.
3.  If the directory is missing a `Project.md`, Magy will ask "What are we building?". Enter your goal, and Magy will generate the project specification for you.

### How It Works

Magy reads the project, plans work, and asks the model what to do.

*   **Read-only actions** (like reading files or listing directories) run automatically.
*   **Restricted actions** (like writing files or executing commands) require explicit operator approval in the UI.
*   **Verification**: After changes are made, Magy runs a verification command (configurable, defaults to `cargo test`). If verification fails, the failure output is given back to the model so it can attempt another fix.

A task is complete when verification succeeds.

## Configuration

Magy uses deterministic bounds found in the coordination logic (and eventually in configuration files):

*   **max-steps**: Maximum reasoning steps per task execution cycle.
*   **max-verifications**: Maximum verification retries per task.

## Models

Models are accessed through a provider interface, allowing different local model backends to be used without changing the core agent.

Magy utilizes **JSON Schema enforcement** to ensure model tool calls are syntactically valid. To ensure reliability with small local models, Magy uses a **flat discriminator schema** that avoids the semantic degradation often caused by complex branching.

## Tools

Magy currently provides:

```text
read_file
write_file
list_directory
discover_files
run_command
task_complete
```

Tool execution is strictly constrained to the selected project directory.

## Building

```text
cargo build
```

Run the tests:

```text
cargo test
```

## License

Magy is licensed under the GNU General Public License v3.0.

See `LICENSE` for the full license text.
