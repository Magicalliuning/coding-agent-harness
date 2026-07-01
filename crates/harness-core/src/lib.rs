pub const V0_CRATE_BOUNDARIES: &[&str] = &[
    "harness-core",
    "harness-events",
    "harness-db",
    "harness-policy",
    "harness-tools",
    "harness-models",
    "harness-context",
    "harness-runtime",
    "harness-cli",
];

pub const PRODUCT_NAME: &str = "Coding Agent Harness";

pub type HarnessResult<T> = Result<T, HarnessError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HarnessError {
    message: String,
}

impl HarnessError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl std::fmt::Display for HarnessError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl std::error::Error for HarnessError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v0_crate_boundaries_match_architecture_decision() {
        assert_eq!(V0_CRATE_BOUNDARIES.len(), 9);
        assert!(V0_CRATE_BOUNDARIES.contains(&"harness-runtime"));
        assert!(!V0_CRATE_BOUNDARIES.contains(&"harness-web"));
        assert!(!V0_CRATE_BOUNDARIES.contains(&"harness-ios"));
        assert!(!V0_CRATE_BOUNDARIES.contains(&"harness-ide"));
    }

    #[test]
    fn harness_error_exposes_message() {
        let error = HarnessError::new("database unavailable");

        assert_eq!(error.message(), "database unavailable");
        assert_eq!(error.to_string(), "database unavailable");
    }
}
