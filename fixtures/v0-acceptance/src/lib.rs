#[cfg(test)]
mod tests {
    #[test]
    fn fake_patch_contains_recovery_marker() {
        let content = std::fs::read_to_string(".harness/fake-agent-turn.md")
            .expect("fake model patch should exist");

        assert!(content.contains("recovered=true"), "missing recovery marker");
    }
}
