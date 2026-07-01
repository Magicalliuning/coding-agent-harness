use std::fs;
use std::path::{Component, Path};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

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

pub const APPLY_FILE_PATCH_TOOL: ToolDescriptor = ToolDescriptor {
    name: "apply_file_patch",
    mutates: true,
    default_decision: PolicyDecision::Ask,
};

pub const CODEX_WORKER_LANE_TOOL: ToolDescriptor = ToolDescriptor {
    name: "codex_cli_worker_lane",
    mutates: true,
    default_decision: PolicyDecision::Ask,
};

#[must_use]
pub const fn verify_command_tool() -> ToolDescriptor {
    VERIFY_COMMAND_TOOL
}

#[must_use]
pub const fn apply_file_patch_tool() -> ToolDescriptor {
    APPLY_FILE_PATCH_TOOL
}

#[must_use]
pub const fn codex_worker_lane_tool() -> ToolDescriptor {
    CODEX_WORKER_LANE_TOOL
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePatchIntent {
    pub repo_path: String,
    pub relative_path: String,
    pub expected_content: Option<String>,
    pub replacement_content: String,
}

impl FilePatchIntent {
    pub fn new(
        repo_path: impl Into<String>,
        relative_path: impl Into<String>,
        expected_content: Option<String>,
        replacement_content: impl Into<String>,
    ) -> HarnessResult<Self> {
        let relative_path = relative_path.into();
        let replacement_content = replacement_content.into();

        if !is_safe_relative_path(&relative_path) {
            return Err(HarnessError::new("patch path must be a safe relative path"));
        }

        if replacement_content.is_empty() {
            return Err(HarnessError::new("patch replacement content is required"));
        }

        Ok(Self {
            repo_path: repo_path.into(),
            relative_path,
            expected_content,
            replacement_content,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePatchObservation {
    pub path: String,
    pub applied: bool,
    pub previous_bytes: usize,
    pub new_bytes: usize,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkerLaneStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
    Rejected,
}

impl WorkerLaneStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
            Self::Rejected => "rejected",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageConfidence {
    Official,
    LocalEstimate,
    Unknown,
}

impl UsageConfidence {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Official => "official",
            Self::LocalEstimate => "local_estimate",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexWorkerLaneBudget {
    pub max_prompt_tokens: usize,
    pub max_output_tokens: usize,
    pub max_stdout_bytes: usize,
}

impl Default for CodexWorkerLaneBudget {
    fn default() -> Self {
        Self {
            max_prompt_tokens: 8_192,
            max_output_tokens: 2_048,
            max_stdout_bytes: 64 * 1024,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexWorkerUsage {
    pub prompt_tokens: Option<usize>,
    pub completion_tokens: Option<usize>,
    pub confidence: UsageConfidence,
}

impl CodexWorkerUsage {
    #[must_use]
    pub const fn unknown() -> Self {
        Self {
            prompt_tokens: None,
            completion_tokens: None,
            confidence: UsageConfidence::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexWorkerLaneIntent {
    pub task: String,
    pub workspace_path: String,
    pub worktree_path: Option<String>,
    pub timeout_ms: u64,
    pub cancellation_requested: bool,
    pub budget: CodexWorkerLaneBudget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexWorkerLaneFixture {
    pub status: WorkerLaneStatus,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub usage: CodexWorkerUsage,
}

impl CodexWorkerLaneFixture {
    #[must_use]
    pub fn succeeded(stdout: impl Into<String>) -> Self {
        Self {
            status: WorkerLaneStatus::Succeeded,
            exit_code: Some(0),
            stdout: stdout.into(),
            stderr: String::new(),
            duration_ms: 0,
            usage: CodexWorkerUsage::unknown(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexWorkerSubprocess {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

impl CodexWorkerSubprocess {
    #[must_use]
    pub fn new(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
            env: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodexWorkerLaneRunner {
    Fixture(CodexWorkerLaneFixture),
    Subprocess(CodexWorkerSubprocess),
}

impl CodexWorkerLaneRunner {
    #[must_use]
    pub fn fixture(fixture: CodexWorkerLaneFixture) -> Self {
        Self::Fixture(fixture)
    }

    #[must_use]
    pub fn subprocess(subprocess: CodexWorkerSubprocess) -> Self {
        Self::Subprocess(subprocess)
    }

    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Fixture(_) => "fixture",
            Self::Subprocess(_) => "subprocess",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodexWorkerLaneObservation {
    pub status: WorkerLaneStatus,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub usage: CodexWorkerUsage,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct OneShotWorkerRunner;

impl OneShotWorkerRunner {
    #[must_use]
    pub fn run(
        self,
        intent: &CodexWorkerLaneIntent,
        subprocess: &CodexWorkerSubprocess,
    ) -> CodexWorkerLaneObservation {
        if intent.cancellation_requested {
            return cancelled_worker_observation();
        }

        let started_at = Instant::now();
        let mut command = Command::new(&subprocess.program);
        command
            .args(&subprocess.args)
            .current_dir(&intent.workspace_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HARNESS_WORKER_TASK", &intent.task)
            .env("HARNESS_WORKER_WORKSPACE", &intent.workspace_path)
            .env(
                "HARNESS_WORKER_WORKTREE",
                intent.worktree_path.as_deref().unwrap_or(""),
            )
            .env("HARNESS_WORKER_TIMEOUT_MS", intent.timeout_ms.to_string())
            .env(
                "HARNESS_WORKER_MAX_STDOUT_BYTES",
                intent.budget.max_stdout_bytes.to_string(),
            );

        for (key, value) in &subprocess.env {
            command.env(key, value);
        }

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                return failed_worker_observation(
                    None,
                    "",
                    format!("worker executable unavailable: {error}"),
                    elapsed_ms(started_at),
                    intent.budget.max_stdout_bytes,
                );
            }
        };

        loop {
            match child.try_wait() {
                Ok(Some(_status)) => {
                    return match child.wait_with_output() {
                        Ok(output) => observation_from_output(
                            output.status.code(),
                            output.stdout,
                            output.stderr,
                            elapsed_ms(started_at),
                            intent.budget.max_stdout_bytes,
                        ),
                        Err(error) => failed_worker_observation(
                            None,
                            "",
                            format!("worker output capture failed: {error}"),
                            elapsed_ms(started_at),
                            intent.budget.max_stdout_bytes,
                        ),
                    };
                }
                Ok(None) => {}
                Err(error) => {
                    let _ = child.kill();
                    return failed_worker_observation(
                        None,
                        "",
                        format!("worker status polling failed: {error}"),
                        elapsed_ms(started_at),
                        intent.budget.max_stdout_bytes,
                    );
                }
            }

            if elapsed_ms(started_at) >= intent.timeout_ms {
                let _ = child.kill();
                return match child.wait_with_output() {
                    Ok(output) => CodexWorkerLaneObservation {
                        status: WorkerLaneStatus::TimedOut,
                        exit_code: None,
                        stdout: truncate_to_bytes(
                            &String::from_utf8_lossy(&output.stdout),
                            intent.budget.max_stdout_bytes,
                        ),
                        stderr: truncate_to_bytes(
                            &String::from_utf8_lossy(&output.stderr),
                            intent.budget.max_stdout_bytes,
                        ),
                        duration_ms: elapsed_ms(started_at),
                        usage: CodexWorkerUsage::unknown(),
                    },
                    Err(error) => failed_worker_observation(
                        None,
                        "",
                        format!("worker timed out and output capture failed: {error}"),
                        elapsed_ms(started_at),
                        intent.budget.max_stdout_bytes,
                    ),
                };
            }

            thread::sleep(Duration::from_millis(5));
        }
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct CodexWorkerLaneFixtureAdapter;

impl CodexWorkerLaneFixtureAdapter {
    #[must_use]
    pub fn run(
        self,
        intent: &CodexWorkerLaneIntent,
        fixture: &CodexWorkerLaneFixture,
    ) -> CodexWorkerLaneObservation {
        if intent.cancellation_requested {
            return CodexWorkerLaneObservation {
                status: WorkerLaneStatus::Cancelled,
                exit_code: None,
                stdout: String::new(),
                stderr: "worker lane cancelled before start".to_owned(),
                duration_ms: 0,
                usage: CodexWorkerUsage::unknown(),
            };
        }

        let status = if fixture.duration_ms > intent.timeout_ms {
            WorkerLaneStatus::TimedOut
        } else {
            fixture.status
        };
        let exit_code = if matches!(
            status,
            WorkerLaneStatus::Cancelled | WorkerLaneStatus::TimedOut
        ) {
            None
        } else {
            fixture.exit_code
        };

        CodexWorkerLaneObservation {
            status,
            exit_code,
            stdout: truncate_to_bytes(&fixture.stdout, intent.budget.max_stdout_bytes),
            stderr: truncate_to_bytes(&fixture.stderr, intent.budget.max_stdout_bytes),
            duration_ms: fixture.duration_ms,
            usage: fixture.usage.clone(),
        }
    }
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

    pub fn run_file_patch(self, intent: &FilePatchIntent) -> HarnessResult<FilePatchObservation> {
        let started_at = Instant::now();
        let repo_root = Path::new(&intent.repo_path)
            .canonicalize()
            .map_err(|error| HarnessError::new(error.to_string()))?;

        if !repo_root.is_dir() {
            return Err(HarnessError::new("repository path must be a directory"));
        }

        let target = repo_root.join(&intent.relative_path);
        let parent = target
            .parent()
            .ok_or_else(|| HarnessError::new("patch target parent is missing"))?;

        fs::create_dir_all(parent).map_err(|error| HarnessError::new(error.to_string()))?;
        let canonical_parent = parent
            .canonicalize()
            .map_err(|error| HarnessError::new(error.to_string()))?;

        if !canonical_parent.starts_with(&repo_root) {
            return Err(HarnessError::new(
                "patch target must stay inside repository",
            ));
        }

        let target_metadata = match fs::symlink_metadata(&target) {
            Ok(metadata) => Some(metadata),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(HarnessError::new(error.to_string())),
        };

        if target_metadata
            .as_ref()
            .is_some_and(|metadata| metadata.file_type().is_symlink())
        {
            return Err(HarnessError::new("patch target cannot be a symlink"));
        }

        let current_content = if target_metadata.is_some() {
            let canonical_target = target
                .canonicalize()
                .map_err(|error| HarnessError::new(error.to_string()))?;

            if !canonical_target.starts_with(&repo_root) {
                return Err(HarnessError::new(
                    "patch target must stay inside repository",
                ));
            }

            Some(
                fs::read_to_string(&canonical_target)
                    .map_err(|error| HarnessError::new(error.to_string()))?,
            )
        } else {
            None
        };

        if intent.expected_content.is_none() && current_content.is_some() {
            return Err(HarnessError::new(
                "patch expected content is required for existing files",
            ));
        }

        if let Some(expected_content) = &intent.expected_content {
            let current_content = current_content
                .as_deref()
                .ok_or_else(|| HarnessError::new("patch expected an existing file"))?;

            if current_content != expected_content {
                return Err(HarnessError::new("patch expected content did not match"));
            }
        }

        let previous_bytes = current_content.as_ref().map_or(0, String::len);
        fs::write(&target, &intent.replacement_content)
            .map_err(|error| HarnessError::new(error.to_string()))?;

        Ok(FilePatchObservation {
            path: intent.relative_path.clone(),
            applied: true,
            previous_bytes,
            new_bytes: intent.replacement_content.len(),
            duration_ms: elapsed_ms(started_at),
        })
    }
}

fn cancelled_worker_observation() -> CodexWorkerLaneObservation {
    CodexWorkerLaneObservation {
        status: WorkerLaneStatus::Cancelled,
        exit_code: None,
        stdout: String::new(),
        stderr: "worker lane cancelled before start".to_owned(),
        duration_ms: 0,
        usage: CodexWorkerUsage::unknown(),
    }
}

fn observation_from_output(
    exit_code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    duration_ms: u64,
    max_stdout_bytes: usize,
) -> CodexWorkerLaneObservation {
    let status = if exit_code == Some(0) {
        WorkerLaneStatus::Succeeded
    } else {
        WorkerLaneStatus::Failed
    };

    CodexWorkerLaneObservation {
        status,
        exit_code,
        stdout: truncate_to_bytes(&String::from_utf8_lossy(&stdout), max_stdout_bytes),
        stderr: truncate_to_bytes(&String::from_utf8_lossy(&stderr), max_stdout_bytes),
        duration_ms,
        usage: CodexWorkerUsage::unknown(),
    }
}

fn failed_worker_observation(
    exit_code: Option<i32>,
    stdout: impl Into<String>,
    stderr: impl Into<String>,
    duration_ms: u64,
    max_stdout_bytes: usize,
) -> CodexWorkerLaneObservation {
    let stdout = stdout.into();
    let stderr = stderr.into();

    CodexWorkerLaneObservation {
        status: WorkerLaneStatus::Failed,
        exit_code,
        stdout: truncate_to_bytes(&stdout, max_stdout_bytes),
        stderr: truncate_to_bytes(&stderr, max_stdout_bytes),
        duration_ms,
        usage: CodexWorkerUsage::unknown(),
    }
}

fn elapsed_ms(started_at: Instant) -> u64 {
    let elapsed = started_at.elapsed().as_millis();
    elapsed.min(u128::from(u64::MAX)) as u64
}

fn is_safe_relative_path(relative_path: &str) -> bool {
    let path = Path::new(relative_path);

    if relative_path.trim().is_empty() || path.is_absolute() {
        return false;
    }

    !path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::Prefix(_) | Component::RootDir
        )
    })
}

fn truncate_to_bytes(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }

    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }

    value[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn verification_tool_is_non_mutating_and_allowed() {
        let tool = verify_command_tool();

        assert_eq!(tool.name, "verify_command");
        assert!(!tool.mutates);
        assert_eq!(tool.default_decision, PolicyDecision::Allow);
    }

    #[test]
    fn file_patch_tool_is_mutating_and_requires_policy() {
        let tool = apply_file_patch_tool();

        assert_eq!(tool.name, "apply_file_patch");
        assert!(tool.mutates);
        assert_eq!(tool.default_decision, PolicyDecision::Ask);
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

    #[test]
    fn file_patch_runtime_creates_safe_repo_file() -> Result<(), Box<dyn std::error::Error>> {
        let repo = fixture_repo()?;
        let intent = FilePatchIntent::new(
            repo.display().to_string(),
            ".harness/fake-agent-turn.md",
            None,
            "fake patch",
        )?;

        let observation = CommandToolRuntime.run_file_patch(&intent)?;
        let written = fs::read_to_string(repo.join(".harness/fake-agent-turn.md"))?;

        assert!(observation.applied);
        assert_eq!(observation.previous_bytes, 0);
        assert_eq!(observation.new_bytes, "fake patch".len());
        assert_eq!(written, "fake patch");

        Ok(())
    }

    #[test]
    fn file_patch_runtime_requires_expected_content_for_existing_files()
    -> Result<(), Box<dyn std::error::Error>> {
        let repo = fixture_repo()?;
        write_file(&repo, "existing.txt", "old")?;
        let intent = FilePatchIntent::new(repo.display().to_string(), "existing.txt", None, "new")?;

        let error = CommandToolRuntime
            .run_file_patch(&intent)
            .expect_err("expected content error");

        assert_eq!(
            error.message(),
            "patch expected content is required for existing files"
        );

        Ok(())
    }

    #[test]
    fn codex_worker_fixture_adapter_records_captured_output_and_usage_confidence() {
        let intent = CodexWorkerLaneIntent {
            task: "draft a patch".to_owned(),
            workspace_path: "C:/repo".to_owned(),
            worktree_path: None,
            timeout_ms: 30_000,
            cancellation_requested: false,
            budget: CodexWorkerLaneBudget {
                max_prompt_tokens: 8_192,
                max_output_tokens: 2_048,
                max_stdout_bytes: 64 * 1024,
            },
        };
        let fixture = CodexWorkerLaneFixture {
            status: WorkerLaneStatus::Succeeded,
            exit_code: Some(0),
            stdout: "codex proposed a patch".to_owned(),
            stderr: String::new(),
            duration_ms: 42,
            usage: CodexWorkerUsage {
                prompt_tokens: Some(120),
                completion_tokens: Some(40),
                confidence: UsageConfidence::LocalEstimate,
            },
        };

        let observation = CodexWorkerLaneFixtureAdapter.run(&intent, &fixture);

        assert_eq!(observation.status, WorkerLaneStatus::Succeeded);
        assert_eq!(observation.stdout, "codex proposed a patch");
        assert_eq!(observation.usage.confidence, UsageConfidence::LocalEstimate);
    }

    #[test]
    fn codex_worker_fixture_adapter_marks_timeout_when_duration_exceeds_contract() {
        let intent = CodexWorkerLaneIntent {
            task: "draft a patch".to_owned(),
            workspace_path: "C:/repo".to_owned(),
            worktree_path: None,
            timeout_ms: 10,
            cancellation_requested: false,
            budget: CodexWorkerLaneBudget {
                max_prompt_tokens: 8_192,
                max_output_tokens: 2_048,
                max_stdout_bytes: 64 * 1024,
            },
        };
        let fixture = CodexWorkerLaneFixture {
            status: WorkerLaneStatus::Succeeded,
            exit_code: Some(0),
            stdout: "late output".to_owned(),
            stderr: String::new(),
            duration_ms: 11,
            usage: CodexWorkerUsage::unknown(),
        };

        let observation = CodexWorkerLaneFixtureAdapter.run(&intent, &fixture);

        assert_eq!(observation.status, WorkerLaneStatus::TimedOut);
        assert_eq!(observation.exit_code, None);
        assert_eq!(observation.usage.confidence, UsageConfidence::Unknown);
    }

    #[test]
    fn codex_worker_fixture_adapter_honors_pre_start_cancellation() {
        let intent = CodexWorkerLaneIntent {
            task: "draft a patch".to_owned(),
            workspace_path: "C:/repo".to_owned(),
            worktree_path: None,
            timeout_ms: 30_000,
            cancellation_requested: true,
            budget: CodexWorkerLaneBudget {
                max_prompt_tokens: 8_192,
                max_output_tokens: 2_048,
                max_stdout_bytes: 64 * 1024,
            },
        };
        let fixture = CodexWorkerLaneFixture {
            status: WorkerLaneStatus::Succeeded,
            exit_code: Some(0),
            stdout: "should not be used".to_owned(),
            stderr: String::new(),
            duration_ms: 42,
            usage: CodexWorkerUsage {
                prompt_tokens: Some(120),
                completion_tokens: Some(40),
                confidence: UsageConfidence::Official,
            },
        };

        let observation = CodexWorkerLaneFixtureAdapter.run(&intent, &fixture);

        assert_eq!(observation.status, WorkerLaneStatus::Cancelled);
        assert_eq!(observation.exit_code, None);
        assert_eq!(observation.stdout, "");
        assert_eq!(observation.usage.confidence, UsageConfidence::Unknown);
    }

    fn fixture_repo() -> Result<PathBuf, Box<dyn std::error::Error>> {
        let suffix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = std::env::temp_dir().join(format!(
            "coding-agent-harness-tools-test-{}-{suffix}",
            std::process::id()
        ));
        fs::create_dir_all(&path)?;
        Ok(path)
    }

    fn write_file(
        root: &Path,
        relative_path: &str,
        content: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let path = root.join(relative_path);

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut file = fs::File::create(path)?;
        file.write_all(content.as_bytes())?;
        Ok(())
    }
}
