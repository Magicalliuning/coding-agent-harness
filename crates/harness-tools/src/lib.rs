use std::fs;
use std::path::{Component, Path};
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

pub const APPLY_FILE_PATCH_TOOL: ToolDescriptor = ToolDescriptor {
    name: "apply_file_patch",
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
