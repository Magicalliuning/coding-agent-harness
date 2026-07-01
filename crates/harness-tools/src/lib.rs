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

#[must_use]
pub const fn verify_command_tool() -> ToolDescriptor {
    VERIFY_COMMAND_TOOL
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verification_tool_is_non_mutating_and_allowed() {
        let tool = verify_command_tool();

        assert_eq!(tool.name, "verify_command");
        assert!(!tool.mutates);
        assert_eq!(tool.default_decision, PolicyDecision::Allow);
    }
}
