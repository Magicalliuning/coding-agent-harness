#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    Ask,
    Deny,
}

impl PolicyDecision {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Ask => "ask",
            Self::Deny => "deny",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyEvaluation {
    pub decision: PolicyDecision,
    pub reason: String,
}

impl PolicyEvaluation {
    #[must_use]
    pub fn new(decision: PolicyDecision, reason: impl Into<String>) -> Self {
        Self {
            decision,
            reason: reason.into(),
        }
    }
}

#[must_use]
pub const fn default_mutation_decision() -> PolicyDecision {
    PolicyDecision::Ask
}

#[must_use]
pub fn evaluate_verification_command(program: &str, args: &[String]) -> PolicyEvaluation {
    let program_name = normalized_program_name(program);

    if program_name.is_empty() {
        return PolicyEvaluation::new(PolicyDecision::Deny, "command program cannot be empty");
    }

    if is_denied_program(program_name) || is_denied_git_command(program_name, args) {
        return PolicyEvaluation::new(
            PolicyDecision::Deny,
            "command is outside the safe verification allowlist",
        );
    }

    if is_allowed_cargo_verification(program_name, args) {
        return PolicyEvaluation::new(
            PolicyDecision::Allow,
            "command matches the safe cargo verification allowlist",
        );
    }

    PolicyEvaluation::new(
        PolicyDecision::Ask,
        "verification command requires approval",
    )
}

fn normalized_program_name(program: &str) -> &str {
    let program_name = std::path::Path::new(program)
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or(program)
        .trim();

    if program_name.to_ascii_lowercase().ends_with(".exe") {
        &program_name[..program_name.len() - 4]
    } else {
        program_name
    }
}

fn is_denied_program(program_name: &str) -> bool {
    matches!(
        program_name.to_ascii_lowercase().as_str(),
        "bash"
            | "cmd"
            | "del"
            | "erase"
            | "powershell"
            | "pwsh"
            | "rd"
            | "rm"
            | "rmdir"
            | "sh"
            | "shutdown"
    )
}

fn is_denied_git_command(program_name: &str, args: &[String]) -> bool {
    if !program_name.eq_ignore_ascii_case("git") {
        return false;
    }

    matches!(args.first().map(String::as_str), Some("clean" | "reset"))
}

fn is_allowed_cargo_verification(program_name: &str, args: &[String]) -> bool {
    if !program_name.eq_ignore_ascii_case("cargo") {
        return false;
    }

    matches!(
        args.first().map(String::as_str),
        Some("--version" | "build" | "clippy" | "fmt" | "test")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutating_actions_default_to_ask() {
        assert_eq!(default_mutation_decision(), PolicyDecision::Ask);
    }

    #[test]
    fn safe_cargo_verification_commands_are_allowed() {
        let args = vec!["test".to_owned(), "--workspace".to_owned()];
        let evaluation = evaluate_verification_command("cargo", &args);

        assert_eq!(evaluation.decision, PolicyDecision::Allow);
        assert_eq!(evaluation.decision.as_str(), "allow");
    }

    #[test]
    fn windows_executable_paths_are_normalized() {
        let args = vec!["--version".to_owned()];
        let evaluation = evaluate_verification_command("C:/tools/CARGO.EXE", &args);

        assert_eq!(evaluation.decision, PolicyDecision::Allow);
    }

    #[test]
    fn dangerous_commands_are_denied() {
        let args = vec!["reset".to_owned(), "--hard".to_owned()];
        let evaluation = evaluate_verification_command("git", &args);

        assert_eq!(evaluation.decision, PolicyDecision::Deny);
        assert_eq!(evaluation.decision.as_str(), "deny");
    }

    #[test]
    fn unknown_verification_commands_require_approval() {
        let args = vec!["--version".to_owned()];
        let evaluation = evaluate_verification_command("rustc", &args);

        assert_eq!(evaluation.decision, PolicyDecision::Ask);
        assert_eq!(evaluation.decision.as_str(), "ask");
    }
}
