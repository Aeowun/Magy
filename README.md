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

On Windows, double-click `launch-magy.bat` to build, start the local app, and
open the browser automatically. For a persistent developer shell, run:

```powershell
.\magy-shell.ps1 -Command test
.\magy-shell.ps1 -Command run
.\magy-shell.ps1 -Command cli -ProjectPath C:\path\to\project
```

Alternatively, double-click `magy-shell.bat` to open a PowerShell window
already rooted at the repository.

### To load a project

1.  Click **Load Project**.
2.  Select a project directory via the native folder picker. Magy will set this as the project root.
3.  If the directory is missing a `Project.md`, Magy will ask "What are we building?". Enter your goal, and Magy will generate the project specification for you.

### How It Works

Magy reads the project, plans work, and asks the model what to do.

*   **Read-only actions** (like reading files or listing directories) run automatically.
*   **Restricted actions** (like writing files or executing commands) require explicit operator approval in the UI.
*   **Commands** are deny-by-default and must exactly match an allowlisted command
    before an approval prompt is shown.
*   Denied actions are recorded in the activity feed and the agent asks the model
    for a replacement action instead of silently ending the run.
*   The workspace includes an **Auto-execute safe tools** toggle. It can
    automatically execute file writes; shell commands remain restricted to the
    exact command allowlist.
*   **Verification**: After changes are made, Magy selects a project-aware verifier:
    `cargo test` for Rust, `npm test` for Node projects, `pytest` for Python
    projects. Projects without a recognized runner cannot be marked complete
    automatically; declare an acceptance command in `Project.md` under a
    `Verification` section, or provide an explicit CLI command.
*   **Verification failures** are warnings with captured output, not runtime
    errors. Magy may retry within its configured bound, but it never treats a
    model claim or generic file scan as acceptance evidence.
*   **Conversational chat** is available from the workspace composer and does
    not start a tool-execution run.

A task is complete when verification succeeds.

An approved write is executed only after the active task is restored in the
agent state. Failed approval resolution is surfaced to the UI and does not
resume the agent.

## Configuration

Magy uses deterministic bounds found in the coordination logic (and eventually in configuration files):

*   **max-steps**: Maximum reasoning steps per task execution cycle.
*   **max-verifications**: Maximum verification retries per task.

The CLI accepts `auto` as the verification command (the default), or an explicit
command when the project requires custom verification:

```text
magy C:\path\to\project auto
magy C:\path\to\project "npm run lint"
```

For projects without a standard test runner, define the acceptance contract
explicitly:

```text
Verification
- node scripts/check-acceptance.js
```

Magy only marks a task complete when this command exits successfully. A model
claim, file scan, or generic static check is not acceptance evidence.

The app rejects overlapping runs, surfaces project/worker failures, retries
malformed structured actions when they contain a JSON-like response, and serves
the bundled UI from an absolute workspace path so launch location does not
change static asset behavior.

The workspace UI includes a persistent activity header, task counts, run state,
quick prompts, multi-line chat input, keyboard send (`Enter`), clearable
activity, responsive mobile layout, and non-blocking error toasts. Chat and
agent activity remain distinct so a conversation does not get buried in raw
tool output.

The workspace also detects the current Git branch and GitHub origin, shows the
number of changed files, and can open the repository on GitHub. This first
integration is intentionally read-only: it does not store credentials, push,
create pull requests, or perform remote mutations.

Workspace panels are collapsible: Project context, Tasks, Source control, and
Execution settings can be folded independently, while the approval card and
chat composer remain available. Activity details can also be collapsed when a
long run produces a dense trace.

The workbench UI uses a VS Code-inspired shell with a navigation rail,
Overview, Activity, Chat, Tasks, Source, and Settings surfaces, plus a
collapsible inspector for step context. Raw agent activity is separated from
conversation, and the Overview is the default project landing surface.

## Models

Models are accessed through a provider interface, allowing different local model backends to be used without changing the core agent.

Magy utilizes **JSON Schema enforcement** to ensure model tool calls are syntactically valid. To ensure reliability with small local models, Magy uses a **flat discriminator schema** that avoids the semantic degradation often caused by complex branching.
The runtime also validates the semantics of that flat shape: each tool may only
populate its applicable fields, required values must be non-empty, and
contradictory requests are rejected as retryable model errors rather than being
silently coerced.
After a successful write, Magy rebuilds the project context before asking for
the next action, so the model sees the current on-disk files instead of a stale
snapshot.

## Tools

Magy currently provides:

```text
read_file
write_file
list_directory
discover_files
git_status
git_diff
run_command
task_complete
```

Tool execution is strictly constrained to the selected project directory.
Writes are size-bounded and use synchronized temporary-file replacement. Command
execution has a wall-clock timeout and bounded captured output.

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
