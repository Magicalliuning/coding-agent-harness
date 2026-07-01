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

#[must_use]
pub fn evaluate_file_patch(relative_path: &str, replacement_bytes: usize) -> PolicyEvaluation {
    if !is_safe_relative_path(relative_path) {
        return PolicyEvaluation::new(
            PolicyDecision::Deny,
            "patch path must stay inside the repository",
        );
    }

    if replacement_bytes > 16 * 1024 {
        return PolicyEvaluation::new(PolicyDecision::Ask, "patch exceeds safe auto-apply size");
    }

    PolicyEvaluation::new(PolicyDecision::Allow, "patch is safe to apply")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodexWorkerLanePolicyInput<'a> {
    pub task: &'a str,
    pub session_repo_path: &'a str,
    pub workspace_path: &'a str,
    pub worktree_path: Option<&'a str>,
    pub timeout_ms: u64,
    pub max_prompt_tokens: usize,
    pub max_output_tokens: usize,
    pub max_stdout_bytes: usize,
}

#[must_use]
pub fn evaluate_codex_worker_lane(input: &CodexWorkerLanePolicyInput<'_>) -> PolicyEvaluation {
    if input.task.trim().is_empty() {
        return PolicyEvaluation::new(
            PolicyDecision::Deny,
            "codex worker lane task cannot be empty",
        );
    }

    if input.task.contains('\0') {
        return PolicyEvaluation::new(
            PolicyDecision::Deny,
            "codex worker lane task contains a nul byte",
        );
    }

    if input.session_repo_path.trim().is_empty() {
        return PolicyEvaluation::new(
            PolicyDecision::Deny,
            "codex worker lane session repo cannot be empty",
        );
    }

    if input.workspace_path.trim().is_empty() {
        return PolicyEvaluation::new(
            PolicyDecision::Deny,
            "codex worker lane workspace cannot be empty",
        );
    }

    if input
        .worktree_path
        .is_some_and(|worktree_path| worktree_path.trim().is_empty())
    {
        return PolicyEvaluation::new(
            PolicyDecision::Deny,
            "codex worker lane worktree cannot be empty",
        );
    }

    if input.workspace_path != input.session_repo_path
        && input.worktree_path != Some(input.workspace_path)
    {
        return PolicyEvaluation::new(
            PolicyDecision::Deny,
            "codex worker lane workspace must match the session repo or explicit worktree",
        );
    }

    if input.timeout_ms == 0 {
        return PolicyEvaluation::new(
            PolicyDecision::Deny,
            "codex worker lane timeout is required",
        );
    }

    if input.max_prompt_tokens == 0 || input.max_output_tokens == 0 || input.max_stdout_bytes == 0 {
        return PolicyEvaluation::new(
            PolicyDecision::Deny,
            "codex worker lane budget must be positive",
        );
    }

    if input.timeout_ms > 30 * 60 * 1000
        || input.max_prompt_tokens > 200_000
        || input.max_output_tokens > 200_000
        || input.max_stdout_bytes > 4 * 1024 * 1024
    {
        return PolicyEvaluation::new(
            PolicyDecision::Ask,
            "codex worker lane exceeds the safe local fixture budget",
        );
    }

    PolicyEvaluation::new(
        PolicyDecision::Allow,
        "codex worker lane contract fits the safe local fixture budget",
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

fn is_safe_relative_path(relative_path: &str) -> bool {
    let path = std::path::Path::new(relative_path);

    if relative_path.trim().is_empty() || path.is_absolute() {
        return false;
    }

    !path.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir
                | std::path::Component::Prefix(_)
                | std::path::Component::RootDir
        )
    })
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

    #[test]
    fn safe_file_patches_are_allowed() {
        let evaluation = evaluate_file_patch(".harness/fake-agent-turn.md", 256);

        assert_eq!(evaluation.decision, PolicyDecision::Allow);
    }

    #[test]
    fn path_traversal_file_patches_are_denied() {
        let evaluation = evaluate_file_patch("../outside.md", 256);

        assert_eq!(evaluation.decision, PolicyDecision::Deny);
    }

    #[test]
    fn large_file_patches_require_approval() {
        let evaluation = evaluate_file_patch(".harness/fake-agent-turn.md", 20 * 1024);

        assert_eq!(evaluation.decision, PolicyDecision::Ask);
    }

    #[test]
    fn valid_codex_worker_lane_contract_is_allowed() {
        let evaluation = evaluate_codex_worker_lane(&CodexWorkerLanePolicyInput {
            task: "summarize the pending patch",
            session_repo_path: "C:/repo",
            workspace_path: "C:/repo",
            worktree_path: None,
            timeout_ms: 30_000,
            max_prompt_tokens: 8_192,
            max_output_tokens: 2_048,
            max_stdout_bytes: 64 * 1024,
        });

        assert_eq!(evaluation.decision, PolicyDecision::Allow);
    }

    #[test]
    fn codex_worker_lane_without_task_is_denied() {
        let evaluation = evaluate_codex_worker_lane(&CodexWorkerLanePolicyInput {
            task: "",
            session_repo_path: "C:/repo",
            workspace_path: "C:/repo",
            worktree_path: None,
            timeout_ms: 30_000,
            max_prompt_tokens: 8_192,
            max_output_tokens: 2_048,
            max_stdout_bytes: 64 * 1024,
        });

        assert_eq!(evaluation.decision, PolicyDecision::Deny);
    }

    #[test]
    fn codex_worker_lane_without_timeout_is_denied() {
        let evaluation = evaluate_codex_worker_lane(&CodexWorkerLanePolicyInput {
            task: "summarize the pending patch",
            session_repo_path: "C:/repo",
            workspace_path: "C:/repo",
            worktree_path: None,
            timeout_ms: 0,
            max_prompt_tokens: 8_192,
            max_output_tokens: 2_048,
            max_stdout_bytes: 64 * 1024,
        });

        assert_eq!(evaluation.decision, PolicyDecision::Deny);
    }

    #[test]
    fn codex_worker_lane_outside_session_repo_is_denied_without_explicit_worktree() {
        let evaluation = evaluate_codex_worker_lane(&CodexWorkerLanePolicyInput {
            task: "summarize the pending patch",
            session_repo_path: "C:/repo",
            workspace_path: "D:/other",
            worktree_path: None,
            timeout_ms: 30_000,
            max_prompt_tokens: 8_192,
            max_output_tokens: 2_048,
            max_stdout_bytes: 64 * 1024,
        });

        assert_eq!(evaluation.decision, PolicyDecision::Deny);
    }

    #[test]
    fn codex_worker_lane_explicit_worktree_is_allowed() {
        let evaluation = evaluate_codex_worker_lane(&CodexWorkerLanePolicyInput {
            task: "summarize the pending patch",
            session_repo_path: "C:/repo",
            workspace_path: "D:/repo-worktree",
            worktree_path: Some("D:/repo-worktree"),
            timeout_ms: 30_000,
            max_prompt_tokens: 8_192,
            max_output_tokens: 2_048,
            max_stdout_bytes: 64 * 1024,
        });

        assert_eq!(evaluation.decision, PolicyDecision::Allow);
    }
}
