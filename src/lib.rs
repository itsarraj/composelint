pub mod parser;
pub mod rules;

pub use rules::{Finding, Severity};

/// Parses and lints raw Compose YAML text in one call.
pub fn lint_compose(content: &str) -> Result<Vec<Finding>, String> {
    let services = parser::parse_services(content)?;
    Ok(rules::lint(&services))
}

#[cfg(test)]
mod fixture_tests {
    use super::*;

    const BAD: &str = include_str!("../fixtures/bad-compose.yml");
    const GOOD: &str = include_str!("../fixtures/good-compose.yml");

    #[test]
    fn bad_fixture_trips_every_rule_at_least_once() {
        let findings = lint_compose(BAD).unwrap();
        let rules: std::collections::HashSet<&str> = findings.iter().map(|f| f.rule).collect();
        for expected in [
            "unpinned-image",
            "no-restart-policy",
            "host-network-mode",
            "privileged-container",
            "unbound-published-port",
        ] {
            assert!(
                rules.contains(expected),
                "expected rule '{expected}' to fire on the bad fixture, got: {rules:?}"
            );
        }
    }

    #[test]
    fn good_fixture_produces_zero_findings() {
        let findings = lint_compose(GOOD).unwrap();
        assert!(
            findings.is_empty(),
            "expected zero findings on the good fixture, got: {findings:?}"
        );
    }
}
