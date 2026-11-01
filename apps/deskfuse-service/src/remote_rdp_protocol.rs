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
/// Windows 本机当前占用的登录会话。同一用户只允许一个会话：远程接管会让
/// 本机锁屏，换成另一个账户则会把当前用户挤下线。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsoleSession {
    pub user: String,
    pub connected: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RdpHost {
    pub port: u16,
    pub certificate_sha256: Option<String>,
    pub checks: Vec<HostCheck>,
    /// 旧版服务不发送该字段；缺失表示无法判断，不应阻止连接。
    #[serde(default)]
    pub console_session: Option<ConsoleSession>,
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
    /// 64×64 PNG, base64. Best effort: extraction can fail for Store apps and
    /// for packages whose icon lives in a resource DLL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon_png: Option<String>,
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
            console_session: None,
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
            console_session: Some(ConsoleSession {
                user: "godd".into(),
                connected: true,
            }),
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
    #[test]
    fn catalog_icon_is_optional_and_round_trips() {
        let with_icon = br#"{"name":"notepad","display_name":"Notepad",
            "path":"C:\\Windows\\notepad.exe","command_line":"","icon_path":"","icon_index":0,
            "icon_png":"aGVsbG8="}"#;
        let app: CatalogApp = serde_json::from_slice(with_icon).unwrap();
        assert_eq!(app.icon_png.as_deref(), Some("aGVsbG8="));
        let encoded = serde_json::to_vec(&app).unwrap();
        assert!(String::from_utf8_lossy(&encoded).contains("icon_png"));

        let without = br#"{"name":"notepad","display_name":"Notepad",
            "path":"C:\\Windows\\notepad.exe","command_line":"","icon_path":"","icon_index":0}"#;
        let app: CatalogApp = serde_json::from_slice(without).unwrap();
        assert_eq!(app.icon_png, None);
        let encoded = serde_json::to_vec(&app).unwrap();
        assert!(!String::from_utf8_lossy(&encoded).contains("icon_png"));
    }

    #[test]
    fn console_session_is_optional_for_old_services() {
        let json = br#"{"port":3389,"certificate_sha256":null,"checks":[]}"#;
        let host: RdpHost = serde_json::from_slice(json).unwrap();
        assert_eq!(host.console_session, None);

        let json = br#"{"port":3389,"certificate_sha256":null,"checks":[],
            "console_session":{"user":"godd","connected":false}}"#;
        let host: RdpHost = serde_json::from_slice(json).unwrap();
        let session = host.console_session.unwrap();
        assert_eq!(session.user, "godd");
        assert!(!session.connected);
    }
}
