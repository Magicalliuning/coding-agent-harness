use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};

use harness_core::{HarnessError, HarnessResult};

pub const AGENTS_FILE: &str = "AGENTS.md";

pub const CONTEXT_MAP_FILE: &str = "CONTEXT-MAP.md";

pub const SKILL_FILE: &str = "SKILL.md";

const DEFAULT_CONTEXT_BUDGET_BYTES: usize = 16 * 1024;
const DEFAULT_CONTEXT_FILE_LIMIT: usize = 8;
const DEFAULT_SKILL_LIMIT: usize = 8;

#[must_use]
pub const fn bootstrap_context_inputs() -> [&'static str; 2] {
    [AGENTS_FILE, CONTEXT_MAP_FILE]
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextBudget {
    pub max_bytes: usize,
    pub max_files: usize,
    pub max_skill_files: usize,
}

impl Default for ContextBudget {
    fn default() -> Self {
        Self {
            max_bytes: DEFAULT_CONTEXT_BUDGET_BYTES,
            max_files: DEFAULT_CONTEXT_FILE_LIMIT,
            max_skill_files: DEFAULT_SKILL_LIMIT,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextCompileRequest {
    pub repo_path: PathBuf,
    pub budget: ContextBudget,
    pub focus_terms: Vec<String>,
}

impl ContextCompileRequest {
    #[must_use]
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
            budget: ContextBudget::default(),
            focus_terms: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextSourceKind {
    RepositoryInstructions,
    ContextMap,
    DomainContext,
}

impl ContextSourceKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RepositoryInstructions => "repository_instructions",
            Self::ContextMap => "context_map",
            Self::DomainContext => "domain_context",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextSource {
    pub kind: ContextSourceKind,
    pub path: String,
    pub content: String,
    pub original_bytes: usize,
    pub included_bytes: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillMetadata {
    pub path: String,
    pub name: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledContextBundle {
    pub repo_path: String,
    pub budget: ContextBudget,
    pub used_bytes: usize,
    pub truncated: bool,
    pub sources: Vec<ContextSource>,
    pub skills: Vec<SkillMetadata>,
}

pub fn compile_repository_context(
    request: ContextCompileRequest,
) -> HarnessResult<CompiledContextBundle> {
    let repo_root = request
        .repo_path
        .canonicalize()
        .map_err(|error| HarnessError::new(error.to_string()))?;

    if !repo_root.is_dir() {
        return Err(HarnessError::new("repository path must be a directory"));
    }

    let focus_terms = normalized_focus_terms(&request.focus_terms);
    let mut bundle = CompiledContextBundle {
        repo_path: repo_root.display().to_string(),
        budget: request.budget,
        used_bytes: 0,
        truncated: false,
        sources: Vec::new(),
        skills: discover_skill_metadata(&repo_root, request.budget.max_skill_files)?,
    };

    add_source_if_present(
        &mut bundle,
        &repo_root,
        Path::new(AGENTS_FILE),
        ContextSourceKind::RepositoryInstructions,
    )?;

    let context_map_content = add_source_if_present(
        &mut bundle,
        &repo_root,
        Path::new(CONTEXT_MAP_FILE),
        ContextSourceKind::ContextMap,
    )?;

    if let Some(context_map_content) = context_map_content {
        add_context_map_sources(&mut bundle, &repo_root, &context_map_content, &focus_terms)?;
    }

    Ok(bundle)
}

fn add_context_map_sources(
    bundle: &mut CompiledContextBundle,
    repo_root: &Path,
    context_map_content: &str,
    focus_terms: &[String],
) -> HarnessResult<()> {
    let mut seen = BTreeSet::new();

    for relative_path in context_paths_from_map(context_map_content) {
        if !seen.insert(relative_path.clone()) {
            continue;
        }

        let Some(path) = safe_relative_path(&relative_path) else {
            continue;
        };

        let absolute_path = repo_root.join(path);
        let Some((absolute_path, content)) = read_repo_file(repo_root, &absolute_path)? else {
            continue;
        };

        if !context_is_relevant(&relative_path, &content, focus_terms) {
            continue;
        }

        add_source_content(
            bundle,
            repo_root,
            &absolute_path,
            ContextSourceKind::DomainContext,
            content,
        )?;
    }

    Ok(())
}

fn add_source_if_present(
    bundle: &mut CompiledContextBundle,
    repo_root: &Path,
    relative_path: &Path,
    kind: ContextSourceKind,
) -> HarnessResult<Option<String>> {
    let path = repo_root.join(relative_path);
    let Some((absolute_path, content)) = read_repo_file(repo_root, &path)? else {
        return Ok(None);
    };

    add_source_content(bundle, repo_root, &absolute_path, kind, content.clone())?;

    Ok(Some(content))
}

fn read_repo_file(repo_root: &Path, path: &Path) -> HarnessResult<Option<(PathBuf, String)>> {
    let Ok(absolute_path) = path.canonicalize() else {
        return Ok(None);
    };

    if !absolute_path.starts_with(repo_root) {
        return Ok(None);
    }

    let content =
        fs::read_to_string(&absolute_path).map_err(|error| HarnessError::new(error.to_string()))?;

    Ok(Some((absolute_path, content)))
}

fn add_source_content(
    bundle: &mut CompiledContextBundle,
    repo_root: &Path,
    path: &Path,
    kind: ContextSourceKind,
    content: String,
) -> HarnessResult<()> {
    if bundle.sources.len() >= bundle.budget.max_files {
        bundle.truncated = true;
        return Ok(());
    }

    let absolute_path = path
        .canonicalize()
        .map_err(|error| HarnessError::new(error.to_string()))?;

    if !absolute_path.starts_with(repo_root) {
        return Ok(());
    }

    let remaining = bundle.budget.max_bytes.saturating_sub(bundle.used_bytes);

    if remaining == 0 {
        bundle.truncated = true;
        return Ok(());
    }

    let included = take_utf8_prefix(&content, remaining);
    let included_bytes = included.len();
    let truncated = included.len() < content.len();
    bundle.used_bytes += included.len();
    bundle.truncated |= truncated;

    bundle.sources.push(ContextSource {
        kind,
        path: relative_display_path(repo_root, &absolute_path),
        content: included,
        original_bytes: content.len(),
        included_bytes,
        truncated,
    });

    Ok(())
}

fn context_paths_from_map(content: &str) -> Vec<String> {
    content
        .lines()
        .flat_map(|line| line.split_whitespace())
        .filter_map(markdown_path_token)
        .collect()
}

fn markdown_path_token(token: &str) -> Option<String> {
    let token = token.trim();
    let path = if let (Some(start), Some(end)) = (token.find('('), token.rfind(')')) {
        &token[start + 1..end]
    } else {
        token
    };
    let path = path.trim_matches(|character: char| {
        matches!(
            character,
            '`' | '"' | '\'' | '*' | '-' | '[' | ']' | '(' | ')' | '<' | '>' | ',' | ';'
        )
    });

    path.ends_with(".md").then(|| path.to_owned())
}

fn safe_relative_path(path: &str) -> Option<PathBuf> {
    let path = Path::new(path);

    if path.is_absolute() {
        return None;
    }

    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::Prefix(_) | Component::RootDir
        )
    }) {
        return None;
    }

    Some(path.to_path_buf())
}

fn context_is_relevant(path: &str, content: &str, focus_terms: &[String]) -> bool {
    if focus_terms.is_empty() {
        return true;
    }

    let path = path.to_ascii_lowercase();
    let content = content.to_ascii_lowercase();

    focus_terms
        .iter()
        .any(|term| path.contains(term) || content.contains(term))
}

fn normalized_focus_terms(focus_terms: &[String]) -> Vec<String> {
    focus_terms
        .iter()
        .map(|term| term.trim().to_ascii_lowercase())
        .filter(|term| !term.is_empty())
        .collect()
}

fn discover_skill_metadata(
    repo_root: &Path,
    max_skill_files: usize,
) -> HarnessResult<Vec<SkillMetadata>> {
    let mut skill_files = Vec::new();

    for root in [".codex/skills", ".agents/skills", "skills"] {
        collect_skill_files(&repo_root.join(root), max_skill_files, &mut skill_files)?;

        if skill_files.len() >= max_skill_files {
            break;
        }
    }

    skill_files
        .into_iter()
        .take(max_skill_files)
        .map(|path| skill_metadata_from_file(repo_root, &path))
        .collect()
}

fn collect_skill_files(
    root: &Path,
    max_skill_files: usize,
    skill_files: &mut Vec<PathBuf>,
) -> HarnessResult<()> {
    if skill_files.len() >= max_skill_files || !root.is_dir() {
        return Ok(());
    }

    let mut entries = fs::read_dir(root)
        .map_err(|error| HarnessError::new(error.to_string()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| HarnessError::new(error.to_string()))?;

    entries.sort_by_key(std::fs::DirEntry::path);

    for entry in entries {
        let path = entry.path();

        if path.is_dir() {
            collect_skill_files(&path, max_skill_files, skill_files)?;
        } else if path.file_name() == Some(OsStr::new(SKILL_FILE)) {
            skill_files.push(path);
        }

        if skill_files.len() >= max_skill_files {
            break;
        }
    }

    Ok(())
}

fn skill_metadata_from_file(repo_root: &Path, path: &Path) -> HarnessResult<SkillMetadata> {
    let content = fs::read_to_string(path).map_err(|error| HarnessError::new(error.to_string()))?;
    let (name, description) = parse_skill_frontmatter(&content);

    Ok(SkillMetadata {
        path: relative_display_path(repo_root, path),
        name,
        description,
    })
}

fn parse_skill_frontmatter(content: &str) -> (Option<String>, Option<String>) {
    let Some(rest) = content.strip_prefix("---") else {
        return (None, None);
    };

    let Some(frontmatter) = rest.split("---").next() else {
        return (None, None);
    };

    let mut name = None;
    let mut description = None;

    for line in frontmatter.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };

        let value = value.trim().trim_matches('"').trim_matches('\'').to_owned();

        match key.trim() {
            "name" => name = Some(value),
            "description" => description = Some(value),
            _ => {}
        }
    }

    (name, description)
}

fn take_utf8_prefix(content: &str, max_bytes: usize) -> String {
    let mut output = String::new();

    for character in content.chars() {
        if output.len() + character.len_utf8() > max_bytes {
            break;
        }

        output.push(character);
    }

    output
}

fn relative_display_path(repo_root: &Path, path: &Path) -> String {
    path.strip_prefix(repo_root)
        .unwrap_or(path)
        .display()
        .to_string()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn bootstrap_context_starts_with_agent_instructions() {
        assert_eq!(bootstrap_context_inputs(), ["AGENTS.md", "CONTEXT-MAP.md"]);
    }

    #[test]
    fn repository_instructions_are_loaded_when_present() -> Result<(), Box<dyn std::error::Error>> {
        let repo = fixture_repo()?;
        write_file(&repo, AGENTS_FILE, "Use the repo instructions.")?;

        let bundle = compile_repository_context(ContextCompileRequest::new(&repo))?;

        assert_eq!(bundle.sources.len(), 1);
        assert_eq!(
            bundle.sources[0].kind,
            ContextSourceKind::RepositoryInstructions
        );
        assert_eq!(bundle.sources[0].path, "AGENTS.md");
        assert!(bundle.sources[0].content.contains("repo instructions"));

        Ok(())
    }

    #[test]
    fn context_map_selects_relevant_contexts() -> Result<(), Box<dyn std::error::Error>> {
        let repo = fixture_repo()?;
        write_file(
            &repo,
            CONTEXT_MAP_FILE,
            "- [Runtime](contexts/runtime/CONTEXT.md)\n- [Billing](contexts/billing/CONTEXT.md)\n",
        )?;
        write_file(
            &repo,
            "contexts/runtime/CONTEXT.md",
            "# Runtime\nThe runtime owns session context.",
        )?;
        write_file(
            &repo,
            "contexts/billing/CONTEXT.md",
            "# Billing\nInvoices and payments.",
        )?;

        let mut request = ContextCompileRequest::new(&repo);
        request.focus_terms = vec!["runtime".to_owned()];
        let bundle = compile_repository_context(request)?;

        assert!(bundle.sources.iter().any(|source| {
            source.kind == ContextSourceKind::ContextMap && source.path == "CONTEXT-MAP.md"
        }));
        assert!(
            bundle
                .sources
                .iter()
                .any(|source| source.path == "contexts/runtime/CONTEXT.md")
        );
        assert!(
            !bundle
                .sources
                .iter()
                .any(|source| source.path == "contexts/billing/CONTEXT.md")
        );

        Ok(())
    }

    #[test]
    fn context_budget_truncates_loaded_content() -> Result<(), Box<dyn std::error::Error>> {
        let repo = fixture_repo()?;
        write_file(&repo, AGENTS_FILE, "abcdefghijklmnopqrstuvwxyz")?;
        let mut request = ContextCompileRequest::new(&repo);
        request.budget = ContextBudget {
            max_bytes: 8,
            max_files: 8,
            max_skill_files: 8,
        };

        let bundle = compile_repository_context(request)?;

        assert_eq!(bundle.used_bytes, 8);
        assert!(bundle.truncated);
        assert_eq!(bundle.sources[0].content, "abcdefgh");
        assert!(bundle.sources[0].truncated);

        Ok(())
    }

    #[test]
    fn skill_metadata_is_discovered_without_running_skills()
    -> Result<(), Box<dyn std::error::Error>> {
        let repo = fixture_repo()?;
        write_file(
            &repo,
            ".codex/skills/demo/SKILL.md",
            "---\nname: demo-skill\ndescription: Compile context fixtures\n---\n# Demo\n",
        )?;

        let bundle = compile_repository_context(ContextCompileRequest::new(&repo))?;

        assert_eq!(bundle.skills.len(), 1);
        assert_eq!(bundle.skills[0].path, ".codex/skills/demo/SKILL.md");
        assert_eq!(bundle.skills[0].name.as_deref(), Some("demo-skill"));
        assert_eq!(
            bundle.skills[0].description.as_deref(),
            Some("Compile context fixtures")
        );

        Ok(())
    }

    fn fixture_repo() -> Result<PathBuf, Box<dyn std::error::Error>> {
        let suffix = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path = std::env::temp_dir().join(format!(
            "coding-agent-harness-context-test-{}-{suffix}",
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
