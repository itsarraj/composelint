//! Just enough of the Compose file schema to lint it — a generic
//! `serde_yaml::Value` walk rather than a full strict-schema
//! deserialization, so a real-world compose file using fields this tool
//! doesn't know about doesn't fail to parse at all.

use serde_yaml::Value;

/// One `services.<name>` entry, with only the fields the rules actually
/// look at pulled out.
#[derive(Debug, Clone, Default)]
pub struct Service {
    pub name: String,
    pub image: Option<String>,
    pub restart: Option<String>,
    pub network_mode: Option<String>,
    pub privileged: bool,
    pub ports: Vec<PortEntry>,
}

/// One `ports:` list entry, normalized out of Compose's two syntaxes
/// (short string form, long mapping form).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortEntry {
    pub raw: String,
    pub host_ip: Option<String>,
    /// `false` for the short bare-port form (`"5432"`, no colon at
    /// all) — that publishes to a random host port on every interface,
    /// a materially different (lower-risk, not developer-chosen) shape
    /// than a fixed host port with no IP restriction, and isn't linted
    /// the same way. See README.
    pub has_explicit_host_port: bool,
}

/// Parses a full Compose YAML document into its services, in
/// declaration order (`serde_yaml::Mapping` preserves insertion order).
pub fn parse_services(content: &str) -> Result<Vec<Service>, String> {
    let doc: Value = serde_yaml::from_str(content).map_err(|e| format!("not valid YAML: {e}"))?;
    let services_val = doc
        .get("services")
        .ok_or_else(|| "no top-level 'services' key found".to_string())?;
    let services_map = services_val
        .as_mapping()
        .ok_or_else(|| "'services' is not a mapping".to_string())?;

    let mut out = Vec::new();
    for (name_val, svc_val) in services_map {
        let name = name_val.as_str().unwrap_or("").to_string();
        if name.is_empty() {
            continue;
        }
        out.push(parse_service(name, svc_val));
    }
    Ok(out)
}

fn parse_service(name: String, svc: &Value) -> Service {
    let image = svc.get("image").and_then(Value::as_str).map(str::to_string);
    let restart = svc
        .get("restart")
        .and_then(Value::as_str)
        .map(str::to_string);
    let network_mode = svc
        .get("network_mode")
        .and_then(Value::as_str)
        .map(str::to_string);
    let privileged = svc
        .get("privileged")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let ports = svc
        .get("ports")
        .and_then(Value::as_sequence)
        .map(|seq| seq.iter().map(parse_port_entry).collect())
        .unwrap_or_default();

    Service {
        name,
        image,
        restart,
        network_mode,
        privileged,
        ports,
    }
}

fn parse_port_entry(v: &Value) -> PortEntry {
    // Long form: {target: 5432, published: 5432, host_ip: "127.0.0.1"}
    if let Some(map) = v.as_mapping() {
        let host_ip = map
            .get(Value::String("host_ip".to_string()))
            .and_then(Value::as_str)
            .map(str::to_string);
        let raw = serde_yaml::to_string(v)
            .unwrap_or_default()
            .trim()
            .to_string();
        return PortEntry {
            raw,
            host_ip,
            has_explicit_host_port: true,
        };
    }

    // Short form: a plain string ("127.0.0.1:5432:5432", "5432:5432",
    // "5432") or (YAML quirk) a bare integer when there's no colon at
    // all, since "5432" with no quotes parses as a number.
    let raw = match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        other => serde_yaml::to_string(other)
            .unwrap_or_default()
            .trim()
            .to_string(),
    };

    let parts: Vec<&str> = split_port_string(&raw);
    match parts.as_slice() {
        [_container_only] => PortEntry {
            raw,
            host_ip: None,
            has_explicit_host_port: false,
        },
        [_host_port, _container_port] => PortEntry {
            raw,
            host_ip: None,
            has_explicit_host_port: true,
        },
        [host_ip, _host_port, _container_port] => PortEntry {
            raw: raw.clone(),
            host_ip: Some(host_ip.to_string()),
            has_explicit_host_port: true,
        },
        _ => PortEntry {
            raw,
            host_ip: None,
            has_explicit_host_port: true,
        },
    }
}

/// Splits a short-form port string on `:`, except for colons nested
/// inside a `${...}` env-var reference — real compose files very
/// commonly write `"${HOST_PORT:-8080}:8080"`, where the `:-` default
/// separator is not a field delimiter. A naive `str::split(':')` here
/// misreads that as three fields (`${HOST_PORT`, `8080}`, `8080`) and
/// mistakes the first chunk for a host-IP restriction — a real false
/// negative caught by running this against an actual Compose file that
/// used exactly this pattern for every published port.
fn split_port_string(raw: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth: i32 = 0;
    let mut start = 0;
    for (i, ch) in raw.char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => depth = (depth - 1).max(0),
            ':' if depth == 0 => {
                parts.push(&raw[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&raw[start..]);
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_basic_service() {
        let yaml = "services:\n  web:\n    image: nginx:1.25\n    restart: unless-stopped\n";
        let services = parse_services(yaml).unwrap();
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].name, "web");
        assert_eq!(services[0].image.as_deref(), Some("nginx:1.25"));
        assert_eq!(services[0].restart.as_deref(), Some("unless-stopped"));
    }

    #[test]
    fn missing_services_key_is_an_error() {
        assert!(parse_services("version: \"3.8\"\n").is_err());
    }

    #[test]
    fn invalid_yaml_is_an_error() {
        assert!(parse_services(": : :not yaml").is_err());
    }

    #[test]
    fn short_form_port_with_host_ip_is_parsed() {
        let entry = parse_port_entry(&Value::String("127.0.0.1:5432:5432".to_string()));
        assert_eq!(entry.host_ip.as_deref(), Some("127.0.0.1"));
        assert!(entry.has_explicit_host_port);
    }

    #[test]
    fn short_form_port_without_host_ip_has_none() {
        let entry = parse_port_entry(&Value::String("5432:5432".to_string()));
        assert_eq!(entry.host_ip, None);
        assert!(entry.has_explicit_host_port);
    }

    #[test]
    fn bare_container_port_short_form_is_not_an_explicit_host_port() {
        // YAML parses an unquoted bare number as an integer, not a string.
        let entry = parse_port_entry(&Value::Number(5432.into()));
        assert!(!entry.has_explicit_host_port);
        assert_eq!(entry.host_ip, None);
    }

    #[test]
    fn long_form_port_with_host_ip_is_parsed() {
        let yaml = "target: 5432\npublished: 5432\nhost_ip: 127.0.0.1\n";
        let v: Value = serde_yaml::from_str(yaml).unwrap();
        let entry = parse_port_entry(&v);
        assert_eq!(entry.host_ip.as_deref(), Some("127.0.0.1"));
    }

    #[test]
    fn long_form_port_without_host_ip_has_none() {
        let yaml = "target: 5432\npublished: 5432\n";
        let v: Value = serde_yaml::from_str(yaml).unwrap();
        let entry = parse_port_entry(&v);
        assert_eq!(entry.host_ip, None);
    }

    #[test]
    fn env_var_default_syntax_in_host_port_is_not_mistaken_for_a_host_ip() {
        // "${RUSTFS_API_PORT:-9000}:9000" is host_port:container_port
        // with the host port coming from an env var with a bash-style
        // default — the ':-' inside the braces must not be treated as
        // a field delimiter, or the ${...} chunk gets misread as a
        // host-IP restriction that was never actually declared.
        let entry = parse_port_entry(&Value::String("${RUSTFS_API_PORT:-9000}:9000".to_string()));
        assert_eq!(
            entry.host_ip, None,
            "the ${{...}} default syntax must not be mistaken for a host-IP restriction"
        );
        assert!(entry.has_explicit_host_port);
    }

    #[test]
    fn env_var_default_syntax_combined_with_a_real_host_ip_still_parses() {
        let entry = parse_port_entry(&Value::String(
            "127.0.0.1:${API_PORT:-9000}:9000".to_string(),
        ));
        assert_eq!(entry.host_ip.as_deref(), Some("127.0.0.1"));
    }

    #[test]
    fn privileged_defaults_to_false_when_absent() {
        let yaml = "services:\n  web:\n    image: nginx:1.25\n";
        let services = parse_services(yaml).unwrap();
        assert!(!services[0].privileged);
    }
}
