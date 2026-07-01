pub use harness_context::ContextBudget;
use harness_context::{CompiledContextBundle, ContextCompileRequest, compile_repository_context};
use harness_core::{HarnessError, HarnessResult};
use harness_db::PostgresEventStore;
use harness_events::{
    ContextCompiledPayload, ContextSourcePayload, EventEnvelope, EventType, FilePatchIntentPayload,
    FilePatchObservationPayload, FilePatchPayload, ModelDecisionPayload, ModelRequestPayload,
    NewEvent, PolicyDecisionPayload, SessionStartedPayload, SkillMetadataPayload,
    ToolCallIntentPayload, ToolObservationPayload,
};
use harness_models::{DeterministicFakeModelProvider, FakeModelRequest, FilePatchProposal};
use harness_policy::{PolicyDecision, evaluate_file_patch, evaluate_verification_command};
use harness_tools::{
    CommandObservation, CommandToolRuntime, FilePatchIntent, FilePatchObservation,
    VerifyCommandIntent,
};
use uuid::Uuid;

pub struct Runtime {
    event_store: PostgresEventStore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartSessionRequest {
    pub repo_path: String,
}

impl StartSessionRequest {
    #[must_use]
    pub fn new(repo_path: impl Into<String>) -> Self {
        Self {
            repo_path: repo_path.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStatus {
    Started,
}

impl SessionStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Started => "started",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionProjection {
    pub session_id: Uuid,
    pub repo_path: String,
    pub status: SessionStatus,
    pub event_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationCommandRequest {
    pub program: String,
    pub args: Vec<String>,
}

impl VerificationCommandRequest {
    #[must_use]
    pub fn new(program: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            program: program.into(),
            args,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerificationCommandResult {
    pub session_id: Uuid,
    pub decision: PolicyDecision,
    pub reason: String,
    pub observation: Option<CommandObservation>,
    pub event_count: usize,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SessionContextCompileRequest {
    pub budget: ContextBudget,
    pub focus_terms: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionContextCompileResult {
    pub session_id: Uuid,
    pub bundle: CompiledContextBundle,
    pub event_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeModelTurnRequest {
    pub task: String,
    pub context: SessionContextCompileRequest,
    pub max_output_tokens: usize,
}

impl FakeModelTurnRequest {
    #[must_use]
    pub fn new(task: impl Into<String>) -> Self {
        Self {
            task: task.into(),
            context: SessionContextCompileRequest::default(),
            max_output_tokens: 256,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeModelTurnResult {
    pub session_id: Uuid,
    pub patch: FilePatchProposal,
    pub decision: PolicyDecision,
    pub reason: String,
    pub observation: Option<FilePatchObservation>,
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub event_count: usize,
}

impl Runtime {
    #[must_use]
    pub fn new(event_store: PostgresEventStore) -> Self {
        Self { event_store }
    }

    pub fn connect_postgres(database_url: &str) -> HarnessResult<Self> {
        Ok(Self::new(PostgresEventStore::connect(database_url)?))
    }

    pub fn start_session(
        &mut self,
        request: StartSessionRequest,
    ) -> HarnessResult<SessionProjection> {
        if request.repo_path.trim().is_empty() {
            return Err(HarnessError::new("repo path cannot be empty"));
        }

        let session_id = Uuid::new_v4();
        let event =
            NewEvent::session_started(session_id, SessionStartedPayload::new(request.repo_path))?;

        self.event_store.append_event(event)?;
        self.show_session(session_id)
    }

    pub fn show_session(&self, session_id: Uuid) -> HarnessResult<SessionProjection> {
        let events = self.event_store.events_for_session(session_id)?;
        project_session(session_id, &events)
    }

    pub fn events_for_session(&self, session_id: Uuid) -> HarnessResult<Vec<EventEnvelope>> {
        self.event_store.events_for_session(session_id)
    }

    pub fn run_verification_command(
        &mut self,
        session_id: Uuid,
        request: VerificationCommandRequest,
    ) -> HarnessResult<VerificationCommandResult> {
        let session = self.show_session(session_id)?;
        let intent =
            VerifyCommandIntent::new(request.program, request.args, session.repo_path.clone())?;
        let tool_name = harness_tools::VERIFY_COMMAND_TOOL.name;

        self.event_store.append_event(NewEvent::tool_call_intended(
            session_id,
            ToolCallIntentPayload::new(
                tool_name,
                intent.program.clone(),
                intent.args.clone(),
                intent.working_dir.clone(),
            ),
        )?)?;

        let evaluation = evaluate_verification_command(&intent.program, &intent.args);

        self.event_store.append_event(NewEvent::policy_decided(
            session_id,
            PolicyDecisionPayload::new(
                tool_name,
                evaluation.decision.as_str(),
                evaluation.reason.clone(),
            ),
        )?)?;

        let observation = if evaluation.decision == PolicyDecision::Allow {
            let observation = CommandToolRuntime.run_verify_command(&intent)?;

            self.event_store
                .append_event(NewEvent::tool_observation_recorded(
                    session_id,
                    ToolObservationPayload::new(
                        tool_name,
                        observation.exit_code,
                        observation.stdout.clone(),
                        observation.stderr.clone(),
                        observation.duration_ms,
                    ),
                )?)?;

            Some(observation)
        } else {
            None
        };

        Ok(VerificationCommandResult {
            session_id,
            decision: evaluation.decision,
            reason: evaluation.reason,
            observation,
            event_count: self.event_store.events_for_session(session_id)?.len(),
        })
    }

    pub fn compile_session_context(
        &mut self,
        session_id: Uuid,
        request: SessionContextCompileRequest,
    ) -> HarnessResult<SessionContextCompileResult> {
        let session = self.show_session(session_id)?;
        let bundle = compile_repository_context(ContextCompileRequest {
            repo_path: session.repo_path.into(),
            budget: request.budget,
            focus_terms: request.focus_terms,
        })?;

        self.event_store.append_event(NewEvent::context_compiled(
            session_id,
            context_compiled_payload(&bundle),
        )?)?;

        Ok(SessionContextCompileResult {
            session_id,
            bundle,
            event_count: self.event_store.events_for_session(session_id)?.len(),
        })
    }

    pub fn run_fake_model_turn(
        &mut self,
        session_id: Uuid,
        request: FakeModelTurnRequest,
    ) -> HarnessResult<FakeModelTurnResult> {
        let context = self.compile_session_context(session_id, request.context)?;
        let model_request = FakeModelRequest::new(
            request.task,
            context.bundle.sources.len(),
            context.bundle.used_bytes,
            request.max_output_tokens,
        )?;
        let provider = DeterministicFakeModelProvider;

        self.event_store
            .append_event(NewEvent::model_request_recorded(
                session_id,
                ModelRequestPayload::new(
                    "deterministic-fake-model",
                    model_request.task.clone(),
                    model_request.context_source_count,
                    model_request.context_used_bytes,
                    model_request.max_output_tokens,
                ),
            )?)?;

        let decision = provider.decide(model_request)?;
        let patch_payload = file_patch_payload(&decision.patch);

        self.event_store
            .append_event(NewEvent::model_decision_recorded(
                session_id,
                ModelDecisionPayload::new(
                    decision.provider,
                    decision.summary.clone(),
                    decision.usage.prompt_tokens,
                    decision.usage.completion_tokens,
                    decision.usage.max_output_tokens,
                    patch_payload.clone(),
                ),
            )?)?;

        let tool_name = harness_tools::APPLY_FILE_PATCH_TOOL.name;
        let intent = FilePatchIntent::new(
            context.bundle.repo_path.clone(),
            decision.patch.path.clone(),
            decision.patch.expected_content.clone(),
            decision.patch.replacement_content.clone(),
        )?;

        self.event_store
            .append_event(NewEvent::file_patch_intended(
                session_id,
                FilePatchIntentPayload::new(
                    tool_name,
                    context.bundle.repo_path.clone(),
                    patch_payload,
                ),
            )?)?;

        let evaluation = evaluate_file_patch(
            &decision.patch.path,
            decision.patch.replacement_content.len(),
        );

        self.event_store.append_event(NewEvent::policy_decided(
            session_id,
            PolicyDecisionPayload::new(
                tool_name,
                evaluation.decision.as_str(),
                evaluation.reason.clone(),
            ),
        )?)?;

        let observation = if evaluation.decision == PolicyDecision::Allow {
            let observation = CommandToolRuntime.run_file_patch(&intent)?;

            self.event_store
                .append_event(NewEvent::file_patch_observation_recorded(
                    session_id,
                    FilePatchObservationPayload::new(
                        tool_name,
                        observation.path.clone(),
                        observation.applied,
                        observation.previous_bytes,
                        observation.new_bytes,
                        observation.duration_ms,
                    ),
                )?)?;

            Some(observation)
        } else {
            None
        };

        Ok(FakeModelTurnResult {
            session_id,
            patch: decision.patch,
            decision: evaluation.decision,
            reason: evaluation.reason,
            observation,
            prompt_tokens: decision.usage.prompt_tokens,
            completion_tokens: decision.usage.completion_tokens,
            event_count: self.event_store.events_for_session(session_id)?.len(),
        })
    }
}

pub fn apply_database_migrations(database_url: &str) -> HarnessResult<()> {
    let store = PostgresEventStore::connect(database_url)?;
    store.apply_migrations()
}

pub fn project_session(
    session_id: Uuid,
    events: &[EventEnvelope],
) -> HarnessResult<SessionProjection> {
    let Some(first_event) = events.first() else {
        return Err(HarnessError::new(format!(
            "session not found: {session_id}"
        )));
    };

    if first_event.event_type != EventType::SessionStarted {
        return Err(HarnessError::new(
            "session replay must start with session.started",
        ));
    }

    let payload: SessionStartedPayload = serde_json::from_value(first_event.payload.clone())
        .map_err(|error| HarnessError::new(error.to_string()))?;

    Ok(SessionProjection {
        session_id,
        repo_path: payload.repo_path,
        status: SessionStatus::Started,
        event_count: events.len(),
    })
}

fn context_compiled_payload(bundle: &CompiledContextBundle) -> ContextCompiledPayload {
    ContextCompiledPayload {
        repo_path: bundle.repo_path.clone(),
        budget_bytes: bundle.budget.max_bytes,
        budget_files: bundle.budget.max_files,
        budget_skill_files: bundle.budget.max_skill_files,
        used_bytes: bundle.used_bytes,
        truncated: bundle.truncated,
        sources: bundle
            .sources
            .iter()
            .map(|source| {
                ContextSourcePayload::new(
                    source.kind.as_str(),
                    source.path.clone(),
                    source.content.clone(),
                    source.original_bytes,
                    source.included_bytes,
                    source.truncated,
                )
            })
            .collect(),
        skills: bundle
            .skills
            .iter()
            .map(|skill| {
                SkillMetadataPayload::new(
                    skill.path.clone(),
                    skill.name.clone(),
                    skill.description.clone(),
                )
            })
            .collect(),
    }
}

fn file_patch_payload(patch: &FilePatchProposal) -> FilePatchPayload {
    FilePatchPayload::new(
        patch.path.clone(),
        patch.expected_content.clone(),
        patch.replacement_content.clone(),
    )
}

#[must_use]
pub fn doctor_report() -> String {
    let context_inputs = harness_context::bootstrap_context_inputs().join(", ");

    format!(
        "{} bootstrap\n- database env: {}\n- migrations: {}\n- eventlog schema: v{}\n- context inputs: {}\n- verification tool: {}",
        harness_core::PRODUCT_NAME,
        harness_db::DATABASE_URL_ENV,
        harness_db::MIGRATIONS_DIR,
        harness_events::eventlog_schema_version(),
        context_inputs,
        harness_tools::VERIFY_COMMAND_TOOL.name
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doctor_report_mentions_runtime_baseline() {
        let report = doctor_report();

        assert!(report.contains("Coding Agent Harness bootstrap"));
        assert!(report.contains("HARNESS_DATABASE_URL"));
        assert!(report.to_ascii_lowercase().contains("eventlog"));
    }

    #[test]
    fn project_session_replays_started_session() {
        let session_id = Uuid::new_v4();
        let event = NewEvent::session_started(session_id, SessionStartedPayload::new("C:/repo"))
            .expect("session started event");
        let envelope = EventEnvelope {
            event_id: event.event_id,
            session_id,
            sequence: 1,
            event_type: event.event_type,
            schema_version: event.schema_version,
            payload: event.payload,
        };

        let projection = project_session(session_id, &[envelope]).expect("projection");

        assert_eq!(projection.session_id, session_id);
        assert_eq!(projection.repo_path, "C:/repo");
        assert_eq!(projection.status, SessionStatus::Started);
        assert_eq!(projection.event_count, 1);
    }

    #[test]
    fn project_session_keeps_count_after_tool_events() {
        let session_id = Uuid::new_v4();
        let session_started =
            NewEvent::session_started(session_id, SessionStartedPayload::new("C:/repo"))
                .expect("session started event");
        let tool_intent = NewEvent::tool_call_intended(
            session_id,
            ToolCallIntentPayload::new(
                "verify_command",
                "cargo",
                vec!["--version".to_owned()],
                "C:/repo",
            ),
        )
        .expect("tool intent event");
        let events = vec![
            EventEnvelope {
                event_id: session_started.event_id,
                session_id,
                sequence: 1,
                event_type: session_started.event_type,
                schema_version: session_started.schema_version,
                payload: session_started.payload,
            },
            EventEnvelope {
                event_id: tool_intent.event_id,
                session_id,
                sequence: 2,
                event_type: tool_intent.event_type,
                schema_version: tool_intent.schema_version,
                payload: tool_intent.payload,
            },
        ];

        let projection = project_session(session_id, &events).expect("projection");

        assert_eq!(projection.event_count, 2);
    }

    #[test]
    fn context_payload_preserves_sources_and_skill_metadata() {
        let bundle = CompiledContextBundle {
            repo_path: "C:/repo".to_owned(),
            budget: ContextBudget {
                max_bytes: 4096,
                max_files: 8,
                max_skill_files: 8,
            },
            used_bytes: 12,
            truncated: false,
            sources: vec![harness_context::ContextSource {
                kind: harness_context::ContextSourceKind::RepositoryInstructions,
                path: "AGENTS.md".to_owned(),
                content: "instructions".to_owned(),
                original_bytes: 12,
                included_bytes: 12,
                truncated: false,
            }],
            skills: vec![harness_context::SkillMetadata {
                path: ".codex/skills/demo/SKILL.md".to_owned(),
                name: Some("demo".to_owned()),
                description: Some("Demo skill".to_owned()),
            }],
        };

        let payload = context_compiled_payload(&bundle);

        assert_eq!(payload.budget_bytes, 4096);
        assert_eq!(payload.budget_files, 8);
        assert_eq!(payload.budget_skill_files, 8);
        assert_eq!(payload.sources[0].path, "AGENTS.md");
        assert_eq!(payload.skills[0].name.as_deref(), Some("demo"));
    }
}
