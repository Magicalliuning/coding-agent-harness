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
}
