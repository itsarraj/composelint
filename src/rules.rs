//! The five misconfigurations this tool exists to catch, one function
//! each, mirroring the structure `dockerlint` uses for its own rules.

use serde::Serialize;

use crate::parser::Service;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub service: String,
    pub rule: &'static str,
    pub severity: Severity,
    pub message: String,
}

/// Runs every rule over every service and returns findings in service
/// declaration order, rule order within a service.
pub fn lint(services: &[Service]) -> Vec<Finding> {
    let mut findings = Vec::new();
    for svc in services {
        findings.extend(check_unpinned_image(svc));
        findings.extend(check_no_restart_policy(svc));
        findings.extend(check_host_network_mode(svc));
        findings.extend(check_privileged(svc));
        findings.extend(check_unbound_published_ports(svc));
    }
    findings
}

/// Rule: `image: nginx` (no tag, implicitly `latest`) or `image:
/// nginx:latest` (explicit but equally unreproducible) — the same
/// footgun as `dockerlint`'s base-image check, applied to a compose
/// service instead of a Dockerfile `FROM`. An image pinned by digest
/// (`@sha256:...`) or referencing a `build:` block instead of `image:`
/// is left alone.
fn check_unpinned_image(svc: &Service) -> Vec<Finding> {
    let Some(image) = &svc.image else {
        return Vec::new();
    };
    if image.contains('$') || image.contains('@') {
        return Vec::new();
    }
    let last_segment = image.rsplit('/').next().unwrap_or(image);
    match last_segment.split_once(':') {
        None => vec![Finding {
            service: svc.name.clone(),
            rule: "unpinned-image",
            severity: Severity::Warning,
            message: format!(
                "image '{image}' has no tag — pin an explicit version instead of floating on whatever 'latest' resolves to today"
            ),
        }],
        Some((_, "latest")) => vec![Finding {
            service: svc.name.clone(),
            rule: "unpinned-image",
            severity: Severity::Warning,
            message: format!("image '{image}' is explicitly pinned to 'latest', which is exactly as unreproducible as no tag at all"),
        }],
        Some(_) => Vec::new(),
    }
}

/// Rule: no `restart` policy set. Any explicit value — including
/// `"no"` — is treated as a deliberate decision and not flagged; only
/// the key's total absence is.
fn check_no_restart_policy(svc: &Service) -> Vec<Finding> {
    if svc.restart.is_some() {
        return Vec::new();
    }
    vec![Finding {
        service: svc.name.clone(),
        rule: "no-restart-policy",
        severity: Severity::Warning,
        message: "no restart policy set — the container won't come back on its own after a crash or a host reboot".to_string(),
    }]
}

/// Rule: `network_mode: host` — the container shares the host's network
/// namespace outright, bypassing whatever isolation the rest of the
/// compose file's networks are providing.
fn check_host_network_mode(svc: &Service) -> Vec<Finding> {
    match &svc.network_mode {
        Some(mode) if mode == "host" => vec![Finding {
            service: svc.name.clone(),
            rule: "host-network-mode",
            severity: Severity::Warning,
            message: "network_mode: host shares the host's network namespace directly, bypassing container network isolation".to_string(),
        }],
        _ => Vec::new(),
    }
}

/// Rule: `privileged: true` — grants effectively all host capabilities,
/// removing most of the point of containerizing the process at all.
fn check_privileged(svc: &Service) -> Vec<Finding> {
    if svc.privileged {
        vec![Finding {
            service: svc.name.clone(),
            rule: "privileged-container",
            severity: Severity::Error,
            message: "privileged: true grants near-full host access — scope down to specific capabilities with cap_add instead".to_string(),
        }]
    } else {
        Vec::new()
    }
}

/// Rule: a published port with a fixed host port and no host-IP
/// restriction (`"5432:5432"`, or `0.0.0.0` spelled out explicitly)
/// binds to every interface on the host, not just loopback — often not
/// what was intended for a database or admin port that only the host
/// itself, or a reverse proxy on the same box, should reach. The bare
/// single-number short form (`"5432"`, no fixed host port) is a
/// materially different, lower-risk shape — Compose picks a random
/// ephemeral host port rather than a developer-chosen fixed one — and
/// is not flagged by this rule.
fn check_unbound_published_ports(svc: &Service) -> Vec<Finding> {
    let mut findings = Vec::new();
    for port in &svc.ports {
        if !port.has_explicit_host_port {
            continue;
        }
        let unbound = match &port.host_ip {
            None => true,
            Some(ip) => ip == "0.0.0.0" || ip.is_empty(),
        };
        if unbound {
            findings.push(Finding {
                service: svc.name.clone(),
                rule: "unbound-published-port",
                severity: Severity::Warning,
                message: format!(
                    "port mapping '{}' publishes to every host interface — bind a specific host IP (e.g. 127.0.0.1) unless this genuinely needs to be reachable from outside the host",
                    port.raw
                ),
            });
        }
    }
    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::PortEntry;

    fn svc(name: &str) -> Service {
        Service {
            name: name.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn flags_image_with_no_tag() {
        let mut s = svc("web");
        s.image = Some("nginx".to_string());
        assert_eq!(check_unpinned_image(&s).len(), 1);
    }

    #[test]
    fn flags_image_with_explicit_latest() {
        let mut s = svc("web");
        s.image = Some("nginx:latest".to_string());
        assert_eq!(check_unpinned_image(&s).len(), 1);
    }

    #[test]
    fn does_not_flag_a_pinned_image_tag() {
        let mut s = svc("web");
        s.image = Some("nginx:1.25.3".to_string());
        assert!(check_unpinned_image(&s).is_empty());
    }

    #[test]
    fn does_not_flag_a_digest_pinned_image() {
        let mut s = svc("web");
        s.image = Some("nginx@sha256:abc123".to_string());
        assert!(check_unpinned_image(&s).is_empty());
    }

    #[test]
    fn does_not_flag_a_service_with_no_image_field() {
        // build: context only, no image: — nothing to check.
        assert!(check_unpinned_image(&svc("web")).is_empty());
    }

    #[test]
    fn flags_missing_restart_policy() {
        assert_eq!(check_no_restart_policy(&svc("web")).len(), 1);
    }

    #[test]
    fn does_not_flag_an_explicit_restart_no() {
        let mut s = svc("web");
        s.restart = Some("no".to_string());
        assert!(check_no_restart_policy(&s).is_empty());
    }

    #[test]
    fn does_not_flag_restart_unless_stopped() {
        let mut s = svc("web");
        s.restart = Some("unless-stopped".to_string());
        assert!(check_no_restart_policy(&s).is_empty());
    }

    #[test]
    fn flags_host_network_mode() {
        let mut s = svc("web");
        s.network_mode = Some("host".to_string());
        assert_eq!(check_host_network_mode(&s).len(), 1);
    }

    #[test]
    fn does_not_flag_bridge_network_mode() {
        let mut s = svc("web");
        s.network_mode = Some("bridge".to_string());
        assert!(check_host_network_mode(&s).is_empty());
    }

    #[test]
    fn flags_privileged_true() {
        let mut s = svc("web");
        s.privileged = true;
        assert_eq!(check_privileged(&s).len(), 1);
        assert_eq!(check_privileged(&s)[0].severity, Severity::Error);
    }

    #[test]
    fn does_not_flag_privileged_false() {
        assert!(check_privileged(&svc("web")).is_empty());
    }

    #[test]
    fn flags_a_published_port_with_no_host_ip() {
        let mut s = svc("db");
        s.ports = vec![PortEntry {
            raw: "5432:5432".to_string(),
            host_ip: None,
            has_explicit_host_port: true,
        }];
        assert_eq!(check_unbound_published_ports(&s).len(), 1);
    }

    #[test]
    fn flags_a_published_port_explicitly_bound_to_all_interfaces() {
        let mut s = svc("db");
        s.ports = vec![PortEntry {
            raw: "0.0.0.0:5432:5432".to_string(),
            host_ip: Some("0.0.0.0".to_string()),
            has_explicit_host_port: true,
        }];
        assert_eq!(check_unbound_published_ports(&s).len(), 1);
    }

    #[test]
    fn does_not_flag_a_port_bound_to_loopback() {
        let mut s = svc("db");
        s.ports = vec![PortEntry {
            raw: "127.0.0.1:5432:5432".to_string(),
            host_ip: Some("127.0.0.1".to_string()),
            has_explicit_host_port: true,
        }];
        assert!(check_unbound_published_ports(&s).is_empty());
    }

    #[test]
    fn does_not_flag_a_bare_container_port_with_no_fixed_host_port() {
        let mut s = svc("db");
        s.ports = vec![PortEntry {
            raw: "5432".to_string(),
            host_ip: None,
            has_explicit_host_port: false,
        }];
        assert!(check_unbound_published_ports(&s).is_empty());
    }

    #[test]
    fn lint_runs_all_rules_across_all_services() {
        let mut bad = svc("web");
        bad.image = Some("nginx".to_string());
        bad.privileged = true;
        let findings = lint(&[bad]);
        assert!(findings.iter().any(|f| f.rule == "unpinned-image"));
        assert!(findings.iter().any(|f| f.rule == "no-restart-policy"));
        assert!(findings.iter().any(|f| f.rule == "privileged-container"));
    }
}
