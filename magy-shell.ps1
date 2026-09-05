param(
    [ValidateSet("build", "test", "run", "cli", "fmt", "check")]
    [string]$Command = "run",
    [string]$ProjectPath = ".",
    [string]$VerificationCommand = "cargo test"
)

$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

switch ($Command) {
    "build" { cargo build --workspace }
    "test" { cargo test --workspace }
    "fmt" { cargo fmt --all }
    "check" { cargo check --workspace }
    "cli" {
        cargo run -p magy-cli -- $ProjectPath $VerificationCommand
    }
    "run" {
        cargo run -p magy-app
    }
}
