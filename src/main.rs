use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use composelint::{lint_compose, Finding, Severity};

#[derive(Parser)]
#[command(
    name = "composelint",
    about = "Lints a docker-compose.yml for common misconfigurations"
)]
struct Cli {
    /// Path to the compose file to lint.
    path: PathBuf,
    /// Emit findings as a JSON array instead of human-readable lines.
    #[arg(long)]
    json: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let content = match fs::read_to_string(&cli.path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("composelint: error: reading {}: {e}", cli.path.display());
            return ExitCode::from(2);
        }
    };

    let findings = match lint_compose(&content) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("composelint: error: {}: {e}", cli.path.display());
            return ExitCode::from(2);
        }
    };

    report(&cli.path.display().to_string(), &findings, cli.json);

    if findings.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

fn report(path: &str, findings: &[Finding], json: bool) {
    if json {
        let out = serde_json::to_string_pretty(findings).unwrap_or_else(|_| "[]".to_string());
        println!("{out}");
        return;
    }
    if findings.is_empty() {
        println!("composelint: {path}: no issues found");
        return;
    }
    for f in findings {
        let sev = match f.severity {
            Severity::Warning => "warning",
            Severity::Error => "error",
        };
        println!("{path}: [{sev}] {}: {} - {}", f.service, f.rule, f.message);
    }
    eprintln!("\ncomposelint: {} issue(s) found in {path}", findings.len());
}
