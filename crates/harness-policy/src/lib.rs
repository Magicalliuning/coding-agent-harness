#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    Ask,
    Deny,
}

#[must_use]
pub const fn default_mutation_decision() -> PolicyDecision {
    PolicyDecision::Ask
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutating_actions_default_to_ask() {
        assert_eq!(default_mutation_decision(), PolicyDecision::Ask);
    }
}
