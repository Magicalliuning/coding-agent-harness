#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelSource {
    Internal,
    ExternalAgentLane,
}

#[must_use]
pub const fn is_harness_owned(source: ModelSource) -> bool {
    matches!(source, ModelSource::Internal)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn internal_model_source_is_harness_owned() {
        assert!(is_harness_owned(ModelSource::Internal));
        assert!(!is_harness_owned(ModelSource::ExternalAgentLane));
    }
}
