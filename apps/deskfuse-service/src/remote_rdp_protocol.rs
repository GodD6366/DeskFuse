//! Versioned Windows catalog contract, kept platform-independent for regression tests.
use serde::{Deserialize, Serialize};
pub const VERSION: u32 = 2;
pub const MAX_REQUEST_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostCheck {
    pub name: String,
    pub passed: bool,
    pub message: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RdpHost {
    pub port: u16,
    pub certificate_sha256: Option<String>,
    pub checks: Vec<HostCheck>,
}
impl RdpHost {
    pub fn available(&self) -> bool {
        self.port > 0
            && self
                .certificate_sha256
                .as_deref()
                .is_some_and(valid_fingerprint)
            && self.checks.iter().all(|check| check.passed)
    }
}
pub fn valid_fingerprint(value: &str) -> bool {
    value.len() == 95
        && value.bytes().enumerate().all(|(i, b)| {
            if i % 3 == 2 {
                b == b':'
            } else {
                b.is_ascii_hexdigit()
            }
        })
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CatalogApp {
    pub name: String,
    pub display_name: String,
    pub path: String,
    pub command_line: String,
    pub icon_path: String,
    pub icon_index: u32,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RemoteAppMessage {
    Catalog {
        version: u32,
        available: bool,
        reason: Option<String>,
        host: RdpHost,
        apps: Vec<CatalogApp>,
    },
    Ready {
        version: u32,
        name: String,
        ticket: Vec<u8>,
        rdp: String,
        host: RdpHost,
    },
    Error {
        version: u32,
        code: String,
        message: String,
    },
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RemoteAppRequest {
    Catalog { version: u32 },
    Prepare { version: u32, name: String },
}
pub fn parse_request(data: &[u8]) -> Result<RemoteAppRequest, &'static str> {
    if data.len() > MAX_REQUEST_BYTES {
        return Err("request_too_large");
    }
    let value: serde_json::Value = serde_json::from_slice(data).map_err(|_| "invalid_request")?;
    if value.get("version").and_then(serde_json::Value::as_u64) != Some(u64::from(VERSION)) {
        return Err("unsupported_version");
    }
    let request: RemoteAppRequest = serde_json::from_value(value).map_err(|_| "invalid_request")?;
    if let RemoteAppRequest::Prepare { name, .. } = &request
        && !crate::remote_app_catalog::valid_name(name)
    {
        return Err("invalid_request");
    }
    Ok(request)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_old_malformed_and_oversized_requests() {
        assert_eq!(
            parse_request(br#"{"type":"launch","version":1,"name":"notepad"}"#).unwrap_err(),
            "unsupported_version"
        );
        assert_eq!(
            parse_request(&vec![b' '; MAX_REQUEST_BYTES + 1]).unwrap_err(),
            "request_too_large"
        );
        assert_eq!(
            parse_request(br#"{"type":"prepare","version":2,"name":"../app"}"#).unwrap_err(),
            "invalid_request"
        );
        assert!(matches!(
            parse_request(br#"{"type":"prepare","version":2,"name":"notepad"}"#),
            Ok(RemoteAppRequest::Prepare { .. })
        ));
    }
    #[test]
    fn readiness_requires_checks_and_a_valid_pin() {
        let mut host = RdpHost {
            port: 3390,
            certificate_sha256: Some(["AB"; 32].join(":")),
            checks: vec![HostCheck {
                name: "rdp_enabled".into(),
                passed: true,
                message: "ok".into(),
            }],
        };
        assert!(host.available());
        host.checks[0].passed = false;
        assert!(!host.available());
        host.checks[0].passed = true;
        host.certificate_sha256 = Some("00:11".into());
        assert!(!host.available());
    }
    #[test]
    fn ready_and_catalog_round_trip() {
        let host = RdpHost {
            port: 3389,
            certificate_sha256: Some(["AA"; 32].join(":")),
            checks: vec![],
        };
        let message = RemoteAppMessage::Ready {
            version: VERSION,
            name: "app".into(),
            ticket: vec![42; 32],
            rdp: "rdp".into(),
            host,
        };
        let json = serde_json::to_vec(&message).unwrap();
        let decoded: RemoteAppMessage = serde_json::from_slice(&json).unwrap();
        assert_eq!(serde_json::to_vec(&decoded).unwrap(), json);
    }
}
