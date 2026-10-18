//! 剪贴板回声账本：认出自写内容和接力改写过的等价表示。
//!
//! 行为参考 BlueSync 的三层抑制（CurrentHostOnly、自有 marker、内容指纹账本），
//! 实现保持在 Rust 引擎内。系统会剥掉自定义 marker type，因此不能只靠
//! `com.deskfuse.clipboard-origin`。本地捕获、对端写入和接力回写共用一张指纹表；
//! 但只有被标记为远端内容的组才抑制再次观察——本地等价再复制必须能在窗口内
//! 重新发布，否则 A→B→A 或失败重试会被回声窗口吞掉。
use deskfuse_core::ClipboardPayload;
use deskfuse_platform::PasteboardSnapshot;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};
use unicode_normalization::UnicodeNormalization;

const TTL: Duration = Duration::from_secs(10);
const CAPACITY: usize = 512;

#[derive(Debug, Default)]
pub(crate) struct ContentLedger {
    last_seen: HashMap<String, Instant>,
    group_of: HashMap<String, u64>,
    members: HashMap<u64, Group>,
    order: VecDeque<u64>,
    next_group: u64,
}

#[derive(Debug, Default)]
struct Group {
    members: Vec<String>,
    /// Set when any member of this group was observed as remote content.
    /// Only a remote-marked group suppresses a re-observation: a local copy
    /// must stay publishable again inside the echo window (A→B→A re-copies,
    /// retries after a send failure), or legitimate re-copies get swallowed.
    remote: bool,
}

impl ContentLedger {
    /// 已见过则刷新整组并入本次标记。返回 true（应抑制）仅当该组被标记为
    /// 远端内容：本地等价再复制命中只刷新、不抑制，允许窗口内再次发布。
    pub(crate) fn observe(&mut self, ids: Vec<String>, remote: bool) -> bool {
        if ids.is_empty() {
            return false;
        }
        self.expire();
        let now = Instant::now();
        let hit = ids.iter().find_map(|id| {
            let gid = *self.group_of.get(id)?;
            let seen = self.last_seen.get(id)?;
            (now.saturating_duration_since(*seen) < TTL).then_some(gid)
        });
        if let Some(gid) = hit {
            self.union(gid, &ids, now, remote);
            self.members.get(&gid).is_some_and(|group| group.remote)
        } else {
            let gid = self.next_group;
            self.next_group = self.next_group.wrapping_add(1);
            self.order.push_back(gid);
            self.union(gid, &ids, now, remote);
            self.evict();
            false
        }
    }

    fn union(&mut self, gid: u64, ids: &[String], now: Instant, remote: bool) {
        let mut group = self.members.remove(&gid).unwrap_or_default();
        group.remote |= remote;
        for id in ids {
            if let Some(&other) = self.group_of.get(id)
                && other != gid
                && let Some(extra) = self.members.remove(&other)
            {
                self.order.retain(|item| *item != other);
                group.remote |= extra.remote;
                for key in extra.members {
                    self.group_of.insert(key.clone(), gid);
                    if !group.members.iter().any(|m| m == &key) {
                        group.members.push(key);
                    }
                }
            }
            if !group.members.iter().any(|m| m == id) {
                group.members.push(id.clone());
            }
            self.group_of.insert(id.clone(), gid);
        }
        for key in &group.members {
            self.last_seen.insert(key.clone(), now);
        }
        self.members.insert(gid, group);
    }

    fn expire(&mut self) {
        let now = Instant::now();
        let stale: Vec<String> = self
            .last_seen
            .iter()
            .filter(|(_, seen)| now.saturating_duration_since(**seen) >= TTL)
            .map(|(id, _)| id.clone())
            .collect();
        for id in stale {
            self.last_seen.remove(&id);
            if let Some(gid) = self.group_of.remove(&id)
                && let Some(group) = self.members.get_mut(&gid)
            {
                group.members.retain(|m| m != &id);
                if group.members.is_empty() {
                    self.members.remove(&gid);
                    self.order.retain(|item| *item != gid);
                }
            }
        }
    }

    fn evict(&mut self) {
        while self.last_seen.len() > CAPACITY {
            let Some(gid) = self.order.pop_front() else {
                break;
            };
            if let Some(group) = self.members.remove(&gid) {
                for id in group.members {
                    self.last_seen.remove(&id);
                    self.group_of.remove(&id);
                }
            }
        }
    }
}

pub(crate) fn snapshot_plain_texts(snapshot: &PasteboardSnapshot) -> Vec<String> {
    let mut texts = Vec::new();
    if let Some(text) = snapshot.text.as_deref().filter(|text| !text.is_empty()) {
        texts.push(text.to_owned());
    }
    for representation in snapshot.items.iter().flat_map(|item| &item.representations) {
        if !matches!(
            representation.type_name.as_str(),
            "public.utf8-plain-text" | "public.plain-text"
        ) {
            continue;
        }
        let Ok(text) = std::str::from_utf8(&representation.data) else {
            continue;
        };
        let text = text.trim_end_matches('\0');
        if !text.is_empty() && !texts.iter().any(|existing| existing == text) {
            texts.push(text.to_owned());
        }
    }
    texts
}

pub(crate) fn snapshot_equivalent_ids(
    snapshot: &PasteboardSnapshot,
    images: &crate::clipboard_image::ClipboardImageFacts,
) -> Vec<String> {
    let mut ids = Vec::new();
    if let Some(digest) = &images.primary {
        ids.push(digest.clone());
    }
    if snapshot.file_urls.is_empty() {
        for text in snapshot_plain_texts(snapshot) {
            ids.extend(text_equivalent_ids(&text));
        }
    }
    ids.extend(images.equivalent_ids.iter().cloned());
    ids.sort();
    ids.dedup();
    ids
}

pub(crate) fn payload_equivalent_ids(payload: &ClipboardPayload) -> Vec<String> {
    match payload.mime.as_str() {
        "text/plain" => String::from_utf8(payload.data.clone())
            .map(|text| text_equivalent_ids(&text))
            .unwrap_or_default(),
        "image/png" => image_equivalent_ids(&payload.data),
        _ => Vec::new(),
    }
}

pub(crate) fn normalize_clipboard_text(text: &str) -> String {
    text.replace("\r\n", "\n")
        .replace('\r', "\n")
        .nfc()
        .collect()
}

fn text_equivalent_ids(text: &str) -> Vec<String> {
    let mut ids = vec![hashed("text", text.as_bytes())];
    let normalized = normalize_clipboard_text(text);
    if normalized != text {
        ids.push(hashed("text", normalized.as_bytes()));
    }
    // 接力回写常补一个尾随换行，不视为另一次复制。
    let trimmed = normalized.trim_end_matches(['\n', '\r']);
    if trimmed != normalized.as_str() {
        ids.push(hashed("text", trimmed.as_bytes()));
    }
    let raw_trimmed = text.trim_end_matches(['\n', '\r']);
    if raw_trimmed != text {
        ids.push(hashed("text", raw_trimmed.as_bytes()));
    }
    ids
}

fn image_equivalent_ids(bytes: &[u8]) -> Vec<String> {
    let mut ids = vec![hashed("img", bytes)];
    if let Some(dhash) = crate::clipboard_image::perceptual_dhash(bytes) {
        ids.push(dhash);
    }
    ids
}

pub(crate) fn hashed(kind: &str, bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(kind.as_bytes());
    digest.update([0]);
    digest.update(bytes);
    format!("{kind}:{}", hex::encode(digest.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_crlf_and_nfc_share_an_alias() {
        let raw_snapshot = PasteboardSnapshot {
            text: Some("a\r\nb".into()),
            ..Default::default()
        };
        let raw = snapshot_equivalent_ids(
            &raw_snapshot,
            &crate::clipboard_image::analyze_snapshot(&raw_snapshot),
        );
        let lf_snapshot = PasteboardSnapshot {
            text: Some("a\nb".into()),
            ..Default::default()
        };
        let lf = snapshot_equivalent_ids(
            &lf_snapshot,
            &crate::clipboard_image::analyze_snapshot(&lf_snapshot),
        );
        assert!(raw.iter().any(|id| lf.contains(id)));
    }

    #[test]
    fn ledger_only_suppresses_groups_marked_as_remote() {
        let mut ledger = ContentLedger::default();
        // 本地首次观察入账；本地等价再观察只刷新、不抑制，
        // 否则 A→B→A 再复制或发送失败后的重试会被回声窗口吞掉。
        assert!(!ledger.observe(text_equivalent_ids("hello\r\n"), false));
        assert!(!ledger.observe(text_equivalent_ids("hello\n"), false));
        assert!(!ledger.observe(text_equivalent_ids("hello"), false));
        // 一旦该组被标记为远端内容，等价出现即回声。
        assert!(ledger.observe(text_equivalent_ids("hello"), true));
        assert!(ledger.observe(text_equivalent_ids("hello\n"), false));
        // 全新内容不受影响。
        assert!(!ledger.observe(text_equivalent_ids("other"), false));
    }

    #[test]
    fn remote_flag_survives_group_merges() {
        let mut ledger = ContentLedger::default();
        assert!(!ledger.observe(vec!["remote-key".into()], true));
        assert!(!ledger.observe(vec!["local-key".into()], false));
        // 与远端组共享等价键会把两组合并；合并后本地键也按远端内容处理。
        assert!(ledger.observe(vec!["local-key".into(), "remote-key".into()], false));
        assert!(ledger.observe(vec!["local-key".into()], false));
    }

    #[test]
    fn utf8_plain_text_item_shares_the_string_alias() {
        let from_string_snapshot = PasteboardSnapshot {
            text: Some("same card".into()),
            ..Default::default()
        };
        let from_string = snapshot_equivalent_ids(
            &from_string_snapshot,
            &crate::clipboard_image::analyze_snapshot(&from_string_snapshot),
        );
        let from_item_snapshot = PasteboardSnapshot {
            items: vec![deskfuse_platform::PasteboardItem {
                representations: vec![deskfuse_platform::PasteboardRepresentation {
                    type_name: "public.utf8-plain-text".into(),
                    data: b"same card\n".to_vec(),
                }],
            }],
            ..Default::default()
        };
        let from_item = snapshot_equivalent_ids(
            &from_item_snapshot,
            &crate::clipboard_image::analyze_snapshot(&from_item_snapshot),
        );
        assert!(from_string.iter().any(|id| from_item.contains(id)));
    }
}
