use harness_core::{HarnessError, HarnessResult};

pub const DETERMINISTIC_FAKE_PROVIDER_KIND: &str = "deterministic_fake";
pub const DETERMINISTIC_FAKE_MODEL_ID: &str = "deterministic-fake-model";
pub const OPENAI_COMPATIBLE_PROVIDER_KIND: &str = "openai_compatible";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelSource {
    Internal,
    ExternalAgentLane,
}

#[must_use]
pub const fn is_harness_owned(source: ModelSource) -> bool {
    matches!(source, ModelSource::Internal)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeModelRequest {
    pub task: String,
    pub context_source_count: usize,
    pub context_used_bytes: usize,
    pub max_output_tokens: usize,
}

impl FakeModelRequest {
    pub fn new(
        task: impl Into<String>,
        context_source_count: usize,
        context_used_bytes: usize,
        max_output_tokens: usize,
    ) -> HarnessResult<Self> {
        let task = task.into();

        if task.trim().is_empty() {
            return Err(HarnessError::new("fake model task cannot be empty"));
        }

        Ok(Self {
            task,
            context_source_count,
            context_used_bytes,
            max_output_tokens,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenUsage {
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub max_output_tokens: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePatchProposal {
    pub path: String,
    pub expected_content: Option<String>,
    pub replacement_content: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeModelDecision {
    pub provider: &'static str,
    pub summary: String,
    pub usage: TokenUsage,
    pub patch: FilePatchProposal,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DeterministicFakeModelProvider;

impl DeterministicFakeModelProvider {
    pub fn decide(self, request: FakeModelRequest) -> HarnessResult<FakeModelDecision> {
        let replacement_content = format!(
            "fake-model-turn\n\ntask={}\ncontext_sources={}\ncontext_used_bytes={}\n",
            request.task.trim(),
            request.context_source_count,
            request.context_used_bytes
        );
        let completion_tokens = estimate_tokens(&replacement_content);

        if completion_tokens > request.max_output_tokens {
            return Err(HarnessError::new(
                "fake model patch exceeds output token budget",
            ));
        }

        Ok(FakeModelDecision {
            provider: DETERMINISTIC_FAKE_MODEL_ID,
            summary: "propose deterministic fixture patch".to_owned(),
            usage: TokenUsage {
                prompt_tokens: estimate_tokens(&request.task)
                    + estimate_tokens_from_bytes(request.context_used_bytes),
                completion_tokens,
                max_output_tokens: request.max_output_tokens,
            },
            patch: FilePatchProposal {
                path: ".harness/fake-agent-turn.md".to_owned(),
                expected_content: None,
                replacement_content,
            },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelProviderSelection {
    DeterministicFake,
    OpenAiCompatible {
        model_id: String,
        api_key_available: bool,
    },
}

impl ModelProviderSelection {
    #[must_use]
    pub fn provider_kind(&self) -> &'static str {
        match self {
            Self::DeterministicFake => DETERMINISTIC_FAKE_PROVIDER_KIND,
            Self::OpenAiCompatible { .. } => OPENAI_COMPATIBLE_PROVIDER_KIND,
        }
    }

    #[must_use]
    pub fn model_id(&self) -> &str {
        match self {
            Self::DeterministicFake => DETERMINISTIC_FAKE_MODEL_ID,
            Self::OpenAiCompatible { model_id, .. } => model_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelProviderRequest {
    pub task: String,
    pub context_source_count: usize,
    pub context_used_bytes: usize,
    pub max_output_tokens: usize,
}

impl ModelProviderRequest {
    pub fn new(
        task: impl Into<String>,
        context_source_count: usize,
        context_used_bytes: usize,
        max_output_tokens: usize,
    ) -> HarnessResult<Self> {
        let task = task.into();

        if task.trim().is_empty() {
            return Err(HarnessError::new("model provider task cannot be empty"));
        }

        Ok(Self {
            task,
            context_source_count,
            context_used_bytes,
            max_output_tokens,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelProviderResponse {
    pub provider_kind: String,
    pub model_id: String,
    pub summary: String,
    pub usage: TokenUsage,
    pub patch: FilePatchProposal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelProviderSkipped {
    pub provider_kind: String,
    pub model_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelProviderOutcome {
    Response(ModelProviderResponse),
    Skipped(ModelProviderSkipped),
}

#[derive(Debug, Default, Clone, Copy)]
pub struct ModelProviderRouter;

impl ModelProviderRouter {
    pub fn route(
        self,
        selection: ModelProviderSelection,
        request: ModelProviderRequest,
    ) -> HarnessResult<ModelProviderOutcome> {
        match selection {
            ModelProviderSelection::DeterministicFake => {
                let fake_request = FakeModelRequest::new(
                    request.task,
                    request.context_source_count,
                    request.context_used_bytes,
                    request.max_output_tokens,
                )?;
                let decision = DeterministicFakeModelProvider.decide(fake_request)?;

                Ok(ModelProviderOutcome::Response(ModelProviderResponse {
                    provider_kind: DETERMINISTIC_FAKE_PROVIDER_KIND.to_owned(),
                    model_id: DETERMINISTIC_FAKE_MODEL_ID.to_owned(),
                    summary: decision.summary,
                    usage: decision.usage,
                    patch: decision.patch,
                }))
            }
            ModelProviderSelection::OpenAiCompatible {
                model_id,
                api_key_available,
            } => {
                if api_key_available {
                    Ok(ModelProviderOutcome::Skipped(ModelProviderSkipped {
                        provider_kind: OPENAI_COMPATIBLE_PROVIDER_KIND.to_owned(),
                        model_id,
                        reason:
                            "openai-compatible provider execution is not implemented in Phase A #75"
                                .to_owned(),
                    }))
                } else {
                    Ok(ModelProviderOutcome::Skipped(ModelProviderSkipped {
                        provider_kind: OPENAI_COMPATIBLE_PROVIDER_KIND.to_owned(),
                        model_id,
                        reason: "openai-compatible provider credentials are not configured"
                            .to_owned(),
                    }))
                }
            }
        }
    }
}

fn estimate_tokens(text: &str) -> usize {
    estimate_tokens_from_bytes(text.len())
}

fn estimate_tokens_from_bytes(bytes: usize) -> usize {
    bytes.div_ceil(4).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_model_source_is_harness_owned() {
        assert!(is_harness_owned(ModelSource::Internal));
        assert!(!is_harness_owned(ModelSource::ExternalAgentLane));
    }

    #[test]
    fn fake_model_proposes_deterministic_patch() -> Result<(), Box<dyn std::error::Error>> {
        let request = FakeModelRequest::new("write a fixture", 2, 128, 128)?;
        let decision = DeterministicFakeModelProvider.decide(request)?;

        assert_eq!(decision.provider, "deterministic-fake-model");
        assert_eq!(decision.patch.path, ".harness/fake-agent-turn.md");
        assert!(
            decision
                .patch
                .replacement_content
                .contains("write a fixture")
        );
        assert!(decision.usage.prompt_tokens > 0);
        assert!(decision.usage.completion_tokens > 0);

        Ok(())
    }

    #[test]
    fn fake_model_respects_output_budget() {
        let request = FakeModelRequest::new("write a fixture", 2, 128, 1).expect("request");
        let error = DeterministicFakeModelProvider
            .decide(request)
            .expect_err("budget error");

        assert_eq!(
            error.message(),
            "fake model patch exceeds output token budget"
        );
    }

    #[test]
    fn router_routes_fake_provider_to_normalized_response() -> Result<(), Box<dyn std::error::Error>>
    {
        let request = ModelProviderRequest::new("write a fixture", 2, 128, 128)?;
        let outcome =
            ModelProviderRouter.route(ModelProviderSelection::DeterministicFake, request)?;
        let ModelProviderOutcome::Response(response) = outcome else {
            panic!("expected response");
        };

        assert_eq!(response.provider_kind, DETERMINISTIC_FAKE_PROVIDER_KIND);
        assert_eq!(response.model_id, DETERMINISTIC_FAKE_MODEL_ID);
        assert_eq!(response.patch.path, ".harness/fake-agent-turn.md");
        assert!(response.usage.prompt_tokens > 0);

        Ok(())
    }

    #[test]
    fn router_returns_skipped_reason_for_unconfigured_real_provider()
    -> Result<(), Box<dyn std::error::Error>> {
        let request = ModelProviderRequest::new("write a fixture", 2, 128, 128)?;
        let outcome = ModelProviderRouter.route(
            ModelProviderSelection::OpenAiCompatible {
                model_id: "gpt-test".to_owned(),
                api_key_available: false,
            },
            request,
        )?;
        let ModelProviderOutcome::Skipped(skipped) = outcome else {
            panic!("expected skipped outcome");
        };

        assert_eq!(skipped.provider_kind, OPENAI_COMPATIBLE_PROVIDER_KIND);
        assert_eq!(skipped.model_id, "gpt-test");
        assert_eq!(
            skipped.reason,
            "openai-compatible provider credentials are not configured"
        );

        Ok(())
    }
}
