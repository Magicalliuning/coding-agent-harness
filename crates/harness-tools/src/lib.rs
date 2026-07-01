use std::process::Command;
use std::time::Instant;

use harness_core::{HarnessError, HarnessResult};
use harness_policy::PolicyDecision;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolDescriptor {
    pub name: &'static str,
    pub mutates: bool,
    pub default_decision: PolicyDecision,
}

pub const VERIFY_COMMAND_TOOL: ToolDescriptor = ToolDescriptor {
    name: "verify_command",
    mutates: false,
    default_decision: PolicyDecision::Allow,
};

#[must_use]
pub const fn verify_command_tool() -> ToolDescriptor {
    VERIFY_COMMAND_TOOL
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifyCommandIntent {
    pub program: String,
    pub args: Vec<String>,
    pub working_dir: String,
}

impl VerifyCommandIntent {
    pub fn new(
        program: impl Into<String>,
        args: Vec<String>,
        working_dir: impl Into<String>,
    ) -> HarnessResult<Self> {
        let program = program.into();

        if program.trim().is_empty() {
            return Err(HarnessError::new(
                "verification command program is required",
            ));
        }

        Ok(Self {
            program,
            args,
            working_dir: working_dir.into(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandObservation {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct CommandToolRuntime;

impl CommandToolRuntime {
    pub fn run_verify_command(
        self,
        intent: &VerifyCommandIntent,
    ) -> HarnessResult<CommandObservation> {
        let started_at = Instant::now();
        let output = Command::new(&intent.program)
            .args(&intent.args)
            .current_dir(&intent.working_dir)
            .output()
            .map_err(|error| HarnessError::new(error.to_string()))?;

        Ok(CommandObservation {
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            duration_ms: elapsed_ms(started_at),
        })
    }
}

fn elapsed_ms(started_at: Instant) -> u64 {
    let elapsed = started_at.elapsed().as_millis();
    elapsed.min(u128::from(u64::MAX)) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verification_tool_is_non_mutating_and_allowed() {
        let tool = verify_command_tool();

        assert_eq!(tool.name, "verify_command");
        assert!(!tool.mutates);
        assert_eq!(tool.default_decision, PolicyDecision::Allow);
    }

    #[test]
    fn verification_command_intent_requires_program() {
        let error = VerifyCommandIntent::new("", Vec::new(), ".").expect_err("empty program");

        assert_eq!(error.message(), "verification command program is required");
    }

    #[test]
    fn command_runtime_captures_status_and_output() -> Result<(), Box<dyn std::error::Error>> {
        let working_dir = std::env::current_dir()?.display().to_string();
        let intent = VerifyCommandIntent::new("rustc", vec!["--version".to_owned()], working_dir)?;
        let observation = CommandToolRuntime.run_verify_command(&intent)?;

        assert_eq!(observation.exit_code, Some(0));
        assert!(observation.stdout.contains("rustc"));

        Ok(())
    }
}
