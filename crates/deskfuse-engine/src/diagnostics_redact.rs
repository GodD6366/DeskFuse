//! 诊断脱敏：诊断会被用户直接贴进公开 Issue，导出前必须去掉能识别用户、
//! 设备和网络位置的信息。
//!
//! 同一次导出内，相同原值总是映射到同一个代号（如 `<device#1>`、`<ip:lan#2>`），
//! 开发者仍能分辨“是不是同一台设备、同一条链路”，但看不到原值。
//! 代号只含 `<>:#` 与字母数字，不会破坏 JSON 字符串。
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

/// 诊断摘要的最大行数：摘要会进入 Issue 链接，必须保持很短。
const SUMMARY_RECENT_PROBLEMS: usize = 20;
const SUMMARY_LINE_LIMIT: usize = 320;
/// 字面量太短时误伤正常文本的概率过高（例如名叫 “Mac” 的电脑）。
const MIN_LITERAL_CHARS: usize = 3;
/// 这些键的值一律视为可识别信息，无论内容长什么样。
const SENSITIVE_KEYS: &[&str] = &[
    "display_name",
    "local_name",
    "device_name",
    "computer_name",
    "hostname",
    "host_name",
    "ssid",
    "bssid",
    "account",
    "user",
    "username",
    "user_name",
    "email",
];

/// 需要整体替换的已知原值。
#[derive(Clone, Debug, Default)]
pub struct RedactionContext {
    /// 本机的显示名、主机名等，统一替换为 `<this-device>`。
    pub local: Vec<String>,
    /// 其他电脑的名称，按出现顺序替换为 `<device#N>`。
    pub peers: Vec<String>,
}

impl RedactionContext {
    /// 补上本进程能自行获知的本机主机名。
    #[must_use]
    pub fn with_host(mut self, hostname: Option<String>) -> Self {
        if let Some(name) = hostname {
            let trimmed = name.trim();
            self.local.push(trimmed.to_owned());
            if let Some(short) = trimmed.strip_suffix(".local") {
                self.local.push(short.to_owned());
            }
        }
        self
    }
}

#[derive(Debug)]
pub struct Redactor {
    /// (原值, 是否本机)，按字符数降序：长名称优先，避免被其中的短名称截断。
    literals: Vec<(String, bool)>,
    pseudonyms: HashMap<String, String>,
    counters: HashMap<&'static str, usize>,
}

impl Redactor {
    #[must_use]
    pub fn new(context: RedactionContext) -> Self {
        let mut literals: Vec<(String, bool)> = Vec::new();
        let local = context.local.into_iter().map(|value| (value, true));
        let peers = context.peers.into_iter().map(|value| (value, false));
        for (value, is_local) in local.chain(peers) {
            let value = value.trim().to_owned();
            // 纯 hex 的设备 ID 由通用规则处理，保持与日志里其他位置的代号一致。
            if value.chars().count() < MIN_LITERAL_CHARS || value == "DeskFuse" || is_hex_id(&value)
            {
                continue;
            }
            if !literals.iter().any(|(existing, _)| *existing == value) {
                literals.push((value, is_local));
            }
        }
        literals.sort_by_key(|(value, _)| std::cmp::Reverse(value.chars().count()));
        Self {
            literals,
            pseudonyms: HashMap::new(),
            counters: HashMap::new(),
        }
    }

    /// 脱敏任意文本；JSONL 逐行仍是合法 JSON。
    pub fn redact(&mut self, text: &str) -> String {
        let mut output = String::with_capacity(text.len());
        let mut index = 0;
        while index < text.len() {
            let rest = &text[index..];
            let previous = text[..index].chars().next_back();
            if let Some((consumed, replacement)) = self.match_literal(rest, previous) {
                output.push_str(&replacement);
                index += consumed;
            } else if let Some((consumed, replacement)) = match_home(rest) {
                output.push_str(replacement);
                index += consumed;
            } else if let Some((consumed, key_part)) = match_sensitive_key(rest, previous) {
                output.push_str(key_part);
                index += consumed;
                let value_length = sensitive_value_length(&text[index..], key_part.ends_with('"'));
                if value_length > 0 {
                    let value = text[index..index + value_length].to_owned();
                    let token = self.literal_token(&value);
                    output.push_str(&token);
                    index += value_length;
                }
            } else if is_word_start(previous) && rest.starts_with(is_token_char) {
                let length = rest
                    .find(|ch: char| !is_token_char(ch))
                    .unwrap_or(rest.len());
                let token = &rest[..length];
                output.push_str(&self.redact_token(token));
                index += length;
            } else {
                let ch = rest.chars().next().expect("index is on a char boundary");
                output.push(ch);
                index += ch.len_utf8();
            }
        }
        output
    }

    fn match_literal(&mut self, rest: &str, previous: Option<char>) -> Option<(usize, String)> {
        if !is_literal_boundary(previous) {
            return None;
        }
        let (value, is_local) = self
            .literals
            .iter()
            .find(|(value, _)| {
                rest.starts_with(value.as_str())
                    && is_literal_boundary(rest[value.len()..].chars().next())
            })
            .cloned()?;
        let token = if is_local {
            "<this-device>".to_owned()
        } else {
            self.pseudonym("device", &value)
        };
        Some((value.len(), token))
    }

    /// 敏感键的值：已知名称沿用同一代号，其余统一记为 `<redacted>`。
    fn literal_token(&mut self, value: &str) -> String {
        match self
            .literals
            .iter()
            .find(|(literal, _)| literal == value)
            .cloned()
        {
            Some((_, true)) => "<this-device>".to_owned(),
            Some((literal, false)) => self.pseudonym("device", &literal),
            None => "<redacted>".to_owned(),
        }
    }

    /// 一个由字母数字与 `.:%@_+-` 组成的片段：判断它是否是邮箱、MAC、IP 或长 ID。
    fn redact_token(&mut self, token: &str) -> String {
        // 句末的标点不属于地址本身。
        let trimmed = token.trim_end_matches(['.', ':', '-', '_']);
        let suffix = &token[trimmed.len()..];
        let replaced = self.classify(trimmed);
        match replaced {
            Some(value) => format!("{value}{suffix}"),
            None => token.to_owned(),
        }
    }

    fn classify(&mut self, token: &str) -> Option<String> {
        if token.is_empty() {
            return None;
        }
        if is_email(token) {
            return Some("<email>".to_owned());
        }
        if is_mac(token) {
            return Some("<mac>".to_owned());
        }
        if let Some(text) = self.redact_address(token) {
            return Some(text);
        }
        if is_hex_id(token) {
            return Some(self.pseudonym("id", &token.to_ascii_lowercase()));
        }
        None
    }

    /// IPv4 / IPv6，可带端口或 `%zone`；端口保留，地址只保留类别。
    fn redact_address(&mut self, token: &str) -> Option<String> {
        // IPv4:port
        if let Some((host, port)) = token.rsplit_once(':')
            && let Ok(ip) = host.parse::<Ipv4Addr>()
            && !port.is_empty()
            && port.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Some(format!("{}:{port}", self.ip_token(IpAddr::V4(ip))));
        }
        if let Ok(ip) = token.parse::<Ipv4Addr>() {
            return Some(self.ip_token(IpAddr::V4(ip)));
        }
        if token.matches(':').count() >= 2 {
            let (address, zone) = token.split_once('%').unwrap_or((token, ""));
            if let Ok(ip) = address.parse::<Ipv6Addr>() {
                // 网卡名（如 en0）不识别用户，保留以便排查走了哪条链路。
                let zone = if zone.is_empty() {
                    String::new()
                } else {
                    format!("%{zone}")
                };
                return Some(format!("{}{zone}", self.ip_token(IpAddr::V6(ip))));
            }
        }
        None
    }

    fn ip_token(&mut self, ip: IpAddr) -> String {
        if ip.is_unspecified() {
            return ip.to_string();
        }
        if ip.is_loopback() {
            return "<ip:loopback>".to_owned();
        }
        let class = if is_local_network(ip) {
            "ip:lan"
        } else {
            "ip:public"
        };
        self.pseudonym(class, &ip.to_string())
    }

    fn pseudonym(&mut self, kind: &'static str, value: &str) -> String {
        let key = format!("{kind}\u{0}{value}");
        if let Some(existing) = self.pseudonyms.get(&key) {
            return existing.clone();
        }
        let counter = self.counters.entry(kind).or_insert(0);
        *counter += 1;
        let token = format!("<{kind}#{counter}>");
        self.pseudonyms.insert(key, token.clone());
        token
    }
}

/// 只依赖文本模式的脱敏：引擎未启动、拿不到设备列表时使用。
pub fn redact_with_host(text: &str, hostname: Option<String>) -> String {
    Redactor::new(RedactionContext::default().with_host(hostname)).redact(text)
}

/// 生成写进 Issue 正文的紧凑摘要。输入应当已经脱敏。
#[must_use]
pub fn summary(redacted_jsonl: &str) -> String {
    let mut counts: Vec<(String, usize)> = Vec::new();
    let mut problems: Vec<String> = Vec::new();
    let mut quality: Vec<String> = Vec::new();
    let mut version = None;
    for line in redacted_jsonl.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let text = |key: &str| {
            value
                .get(key)
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
        };
        let subsystem = text("subsystem").to_owned();
        if version.is_none() && !text("version").is_empty() {
            version = Some(text("version").to_owned());
        }
        match counts.iter_mut().find(|(name, _)| *name == subsystem) {
            Some((_, count)) => *count += 1,
            None => counts.push((subsystem.clone(), 1)),
        }
        if matches!(text("level"), "Warning" | "Error") {
            let mut entry = format!(
                "{} {} {}/{}",
                text("timestamp"),
                text("level"),
                subsystem,
                text("event")
            );
            for key in ["error_code", "result"] {
                if !text(key).is_empty() {
                    entry.push_str(&format!(" {key}={}", text(key)));
                }
            }
            problems.push(clip(&entry));
        } else if subsystem == "input_quality" {
            quality.push(clip(line));
        }
    }
    let mut lines = vec![format!(
        "version={}",
        version.unwrap_or_else(|| "unknown".to_owned())
    )];
    counts.sort_by_key(|(_, count)| std::cmp::Reverse(*count));
    lines.push(format!(
        "records: {}",
        counts
            .iter()
            .map(|(name, count)| format!("{name}={count}"))
            .collect::<Vec<_>>()
            .join(" ")
    ));
    lines.extend(quality.into_iter().rev().take(2));
    let skip = problems.len().saturating_sub(SUMMARY_RECENT_PROBLEMS);
    lines.extend(problems.into_iter().skip(skip));
    lines.join("\n")
}

fn clip(text: &str) -> String {
    if text.chars().count() <= SUMMARY_LINE_LIMIT {
        return text.to_owned();
    }
    let mut clipped: String = text.chars().take(SUMMARY_LINE_LIMIT).collect();
    clipped.push('…');
    clipped
}

fn is_local_network(ip: IpAddr) -> bool {
    match ip {
        // 100.64.0.0/10 是运营商级 NAT 与 Tailscale 等虚拟局域网常用的地址段。
        IpAddr::V4(ip) => {
            ip.is_private()
                || ip.is_link_local()
                || (ip.octets()[0] == 100 && (64..128).contains(&ip.octets()[1]))
        }
        IpAddr::V6(ip) => {
            let first = ip.segments()[0];
            (first & 0xfe00) == 0xfc00 || (first & 0xffc0) == 0xfe80
        }
    }
}

fn is_token_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || matches!(ch, '.' | ':' | '%' | '@' | '_' | '+' | '-')
}

/// 片段只能从单词边界开始，避免把长单词的后半截当成地址。
fn is_word_start(previous: Option<char>) -> bool {
    previous.is_none_or(|ch| !is_token_char(ch))
}

/// 名称两侧不能紧挨 ASCII 字母数字，避免 “Mac” 误伤 “MacBook”。
fn is_literal_boundary(neighbor: Option<char>) -> bool {
    neighbor.is_none_or(|ch| !ch.is_ascii_alphanumeric() && ch != '_')
}

fn is_hex_id(token: &str) -> bool {
    // 至少含一个字母：纯数字的长串通常是时间戳或计数器，不是标识。
    token.len() >= 16
        && token.bytes().all(|byte| byte.is_ascii_hexdigit())
        && token.bytes().any(|byte| byte.is_ascii_alphabetic())
}

fn is_mac(token: &str) -> bool {
    for separator in [':', '-'] {
        let parts: Vec<_> = token.split(separator).collect();
        if parts.len() == 6
            && parts
                .iter()
                .all(|part| part.len() == 2 && part.bytes().all(|byte| byte.is_ascii_hexdigit()))
        {
            return true;
        }
    }
    false
}

fn is_email(token: &str) -> bool {
    let Some((local, domain)) = token.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && domain
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".-".contains(&byte))
}

/// 用户目录：`/Users/<name>`、`/home/<name>`、`C:\Users\<name>`（含 JSON 转义形式）统一替换为 `~`。
fn match_home(rest: &str) -> Option<(usize, &'static str)> {
    let prefix_length = if rest.starts_with("/Users/") {
        "/Users/".len()
    } else if rest.starts_with("/home/") {
        "/home/".len()
    } else {
        let bytes = rest.as_bytes();
        if bytes.len() > 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
            let after_drive = &rest[2..];
            ["\\\\Users\\\\", "\\Users\\", "/Users/"]
                .iter()
                .find(|separator| {
                    after_drive
                        .get(..separator.len())
                        .is_some_and(|head| head.eq_ignore_ascii_case(separator))
                })
                .map(|separator| 2 + separator.len())?
        } else {
            return None;
        }
    };
    let name = &rest[prefix_length..];
    let length = name
        .find(['/', '\\', '"', '\'', ' ', '\n', ',', ')'])
        .unwrap_or(name.len());
    // `/Users/Shared` 之类的公共目录不含用户名。
    if length == 0 || name[..length].eq_ignore_ascii_case("shared") {
        return None;
    }
    Some((prefix_length + length, "~"))
}

/// `display_name=` 或 `"display_name":"` 形式的敏感键，返回已消费长度与需要原样保留的键部分。
fn match_sensitive_key(rest: &str, previous: Option<char>) -> Option<(usize, &str)> {
    if rest.starts_with('"') {
        let key = SENSITIVE_KEYS.iter().find(|key| {
            rest[1..].starts_with(*key) && rest[1 + key.len()..].starts_with("\":\"")
        })?;
        let length = 1 + key.len() + 3;
        return Some((length, &rest[..length]));
    }
    if !is_word_start(previous) {
        return None;
    }
    let key = SENSITIVE_KEYS
        .iter()
        .find(|key| rest.starts_with(*key) && rest[key.len()..].starts_with('='))?;
    let length = key.len() + 1;
    Some((length, &rest[..length]))
}

/// 值的范围：JSON 字符串到未转义的引号为止；`key=value` 形式到分隔符为止（名称里允许空格）。
fn sensitive_value_length(rest: &str, quoted: bool) -> usize {
    if quoted {
        let mut escaped = false;
        for (index, ch) in rest.char_indices() {
            match ch {
                '\\' if !escaped => escaped = true,
                '"' if !escaped => return index,
                _ => escaped = false,
            }
        }
        return rest.len();
    }
    // `Some("…")` 之类的 Debug 输出：引号里的内容整体作为值。
    let rest_trimmed = rest
        .strip_prefix("Some(\\\"")
        .or_else(|| rest.strip_prefix("Some(\""));
    if let Some(inner) = rest_trimmed {
        let prefix = rest.len() - inner.len();
        let end = inner.find(['\\', '"']).unwrap_or(inner.len());
        return prefix + end;
    }
    rest.find(['"', ',', ';', ')', '}', '\n', '&', '\\'])
        .unwrap_or(rest.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn redactor() -> Redactor {
        Redactor::new(RedactionContext {
            local: vec!["godd 的 MacBook Pro".into()],
            peers: vec!["Office-PC".into(), "Mac".into(), "ab".into()],
        })
    }

    #[test]
    fn replaces_known_names_with_stable_pseudonyms() {
        let mut redactor = redactor();
        let output =
            redactor.redact("from godd 的 MacBook Pro to Office-PC, again Office-PC; MacBook Air");
        assert_eq!(
            output,
            "from <this-device> to <device#1>, again <device#1>; MacBook Air"
        );
    }

    #[test]
    fn literal_requires_word_boundary() {
        let mut redactor = redactor();
        assert_eq!(
            redactor.redact("Mac MacBook iMac"),
            "<device#1> MacBook iMac"
        );
    }

    #[test]
    fn ips_keep_class_and_port() {
        let mut redactor = redactor();
        let output = redactor.redact(
            "endpoint=192.168.1.20:42424 peer 192.168.1.20 public 8.8.8.8 lo 127.0.0.1 any 0.0.0.0 v6 fe80::1%en0 tail 10.0.0.1.",
        );
        assert_eq!(
            output,
            "endpoint=<ip:lan#1>:42424 peer <ip:lan#1> public <ip:public#1> lo <ip:loopback> any 0.0.0.0 v6 <ip:lan#2>%en0 tail <ip:lan#3>."
        );
    }

    #[test]
    fn hex_ids_mac_and_email() {
        let mut redactor = redactor();
        let id = "a3f9c2e81b7d4f6e9a0b1c2d3e4f5a6b";
        let output = redactor.redact(&format!(
            "peer={id} again {} mac aa:bb:cc:dd:ee:ff mail a.b@example.com ts 1729580000123456789",
            id.to_uppercase()
        ));
        assert_eq!(
            output,
            "peer=<id#1> again <id#1> mac <mac> mail <email> ts 1729580000123456789"
        );
    }

    #[test]
    fn keeps_versions_uuids_and_timestamps() {
        let mut redactor = redactor();
        let text =
            "version 0.7.0 request 3f2a1b4c-1d2e-4f5a-9b8c-7d6e5f4a3b2c at 1729580000.123 en0";
        assert_eq!(redactor.redact(text), text);
    }

    #[test]
    fn home_directories_become_tilde() {
        let mut redactor = redactor();
        assert_eq!(
            redactor.redact(r#"/Users/godd/Library/x /home/alice/a C:\Users\Bob\AppData C:\\Users\\Bob\\x /Users/Shared/y"#),
            r#"~/Library/x ~/a ~\AppData ~\\x /Users/Shared/y"#
        );
    }

    #[test]
    fn sensitive_keys_hide_unknown_values() {
        let mut redactor = redactor();
        let output = redactor.redact(r#"local_device_id=x, display_name=Someone's iMac","ssid":"Home \"5G\"","hostname":"Office-PC" reason=Some(\"display_name=Private\")"#);
        assert_eq!(
            output,
            r#"local_device_id=x, display_name=<redacted>","ssid":"<redacted>","hostname":"<device#1>" reason=Some(\"display_name=<redacted>\")"#
        );
    }

    #[test]
    fn jsonl_stays_valid() {
        let mut redactor = redactor();
        let line = serde_json::json!({
            "subsystem": "runtime", "result": "endpoint=10.0.0.2:1 peer=a3f9c2e81b7d4f6e9a0b display_name=godd 的 MacBook Pro",
            "local_addr": "[fe80::1%en0]:5000", "path": "/Users/godd/Desktop/a.txt"
        })
        .to_string();
        let output = redactor.redact(&line);
        let parsed: serde_json::Value = serde_json::from_str(&output).expect("valid JSON");
        let text = parsed.to_string();
        for leaked in ["10.0.0.2", "fe80", "godd", "MacBook", "a3f9c2e8"] {
            assert!(!text.contains(leaked), "{leaked} leaked: {text}");
        }
    }

    #[test]
    fn host_only_redaction_strips_local_suffix() {
        assert_eq!(
            redact_with_host("host Studio.local and Studio", Some("Studio.local".into())),
            "host <this-device> and <this-device>"
        );
    }

    #[test]
    fn summary_lists_problems_and_counts() {
        let jsonl = [
            r#"{"subsystem":"runtime","version":"0.7.0","level":"Info","event":"start","timestamp":"1"}"#,
            r#"{"subsystem":"runtime","level":"Error","event":"connect","timestamp":"2","error_code":"timeout","result":"endpoint=<ip:lan#1>:1"}"#,
            r#"{"subsystem":"input_quality","stage":"x"}"#,
            "not json",
        ]
        .join("\n");
        let summary = summary(&jsonl);
        assert!(summary.starts_with("version=0.7.0\nrecords: runtime=2 input_quality=1"));
        assert!(
            summary.contains(
                "2 Error runtime/connect error_code=timeout result=endpoint=<ip:lan#1>:1"
            )
        );
        assert!(summary.contains(r#"{"subsystem":"input_quality","stage":"x"}"#));
    }
}
