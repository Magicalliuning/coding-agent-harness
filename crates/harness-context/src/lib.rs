pub const AGENTS_FILE: &str = "AGENTS.md";

pub const CONTEXT_MAP_FILE: &str = "CONTEXT-MAP.md";

#[must_use]
pub const fn bootstrap_context_inputs() -> [&'static str; 2] {
    [AGENTS_FILE, CONTEXT_MAP_FILE]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bootstrap_context_starts_with_agent_instructions() {
        assert_eq!(bootstrap_context_inputs(), ["AGENTS.md", "CONTEXT-MAP.md"]);
    }
}
