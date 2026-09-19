//! 剪贴板回声账本：认出自写内容和接力改写过的等价表示。
//!
//! 行为参考 BlueSync 的三层抑制（CurrentHostOnly、自有 marker、内容指纹账本），
//! 实现保持在 Rust 引擎内。系统会剥掉自定义 marker type，因此不能只靠
//! `com.unidesk.clipboard-origin`。本地捕获、对端写入和接力回写共用一张指纹表；
//! 命中任一等价键即视为回声。
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};
use unicode_normalization::UnicodeNormalization;
use unidesk_core::ClipboardPayload;
use unidesk_platform::PasteboardSnapshot;

const TTL: Duration = Duration::from_secs(10);
const CAPACITY: usize = 512;

#[derive(Debug, Default)]
pub(crate) struct ContentLedger {
    last_seen: HashMap<String, Instant>,
    group_of: HashMap<String, u64>,
    members: HashMap<u64, Vec<String>>,
    order: VecDeque<u64>,
    next_group: u64,
}

impl ContentLedger {
    /// 已见过则刷新整组并返回 true（应抑制）；否则入账并返回 false。
    pub(crate) fn observe(&mut self, ids: Vec<String>) -> bool {
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
            self.union(gid, &ids, now);
            true
        } else {
            let gid = self.next_group;
            self.next_group = self.next_group.wrapping_add(1);
            self.order.push_back(gid);
            self.union(gid, &ids, now);
            self.evict();
            false
        }
    }

    fn union(&mut self, gid: u64, ids: &[String], now: Instant) {
        let mut members = self.members.remove(&gid).unwrap_or_default();
        for id in ids {
            if let Some(&other) = self.group_of.get(id)
                && other != gid
                && let Some(extra) = self.members.remove(&other)
            {
                self.order.retain(|item| *item != other);
                for key in extra {
                    self.group_of.insert(key.clone(), gid);
                    if !members.iter().any(|m| m == &key) {
                        members.push(key);
                    }
                }
            }
            if !members.iter().any(|m| m == id) {
                members.push(id.clone());
            }
            self.group_of.insert(id.clone(), gid);
        }
        for key in &members {
            self.last_seen.insert(key.clone(), now);
        }
        self.members.insert(gid, members);
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
                && let Some(members) = self.members.get_mut(&gid)
            {
                members.retain(|m| m != &id);
                if members.is_empty() {
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
            if let Some(members) = self.members.remove(&gid) {
                for id in members {
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

pub(crate) fn snapshot_equivalent_ids(snapshot: &PasteboardSnapshot) -> Vec<String> {
    let mut ids = Vec::new();
    if let Some(digest) = crate::clipboard_image::snapshot_digest(snapshot) {
        ids.push(digest);
    }
    if snapshot.file_urls.is_empty() {
        for text in snapshot_plain_texts(snapshot) {
            ids.extend(text_equivalent_ids(&text));
        }
    }
    if let Some(png) = snapshot.png.as_deref() {
        ids.extend(image_equivalent_ids(png));
    }
    for rep in snapshot.items.iter().flat_map(|item| &item.representations) {
        if matches!(
            rep.type_name.as_str(),
            "public.png" | "public.tiff" | "public.jpeg" | "public.heic"
        ) {
            ids.extend(image_equivalent_ids(&rep.data));
        }
    }
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
    if let Some(dhash) = perceptual_dhash(bytes) {
        ids.push(dhash);
    }
    ids
}

fn hashed(kind: &str, bytes: &[u8]) -> String {
    let mut digest = Sha256::new();
    digest.update(kind.as_bytes());
    digest.update([0]);
    digest.update(bytes);
    format!("{kind}:{}", hex::encode(digest.finalize()))
}

/// 8×8 邻域 dHash。只作接力改写识别，不入库、不写日志。
fn perceptual_dhash(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || bytes.len() > 8 * 1024 * 1024 {
        return None;
    }
    let image = image::load_from_memory(bytes).ok()?.to_luma8();
    let small = image::imageops::resize(&image, 9, 8, image::imageops::FilterType::Triangle);
    let mut bits: u64 = 0;
    for y in 0..8 {
        for x in 0..8 {
            if small.get_pixel(x, y).0[0] > small.get_pixel(x + 1, y).0[0] {
                bits |= 1 << (y * 8 + x);
            }
        }
    }
    Some(format!("dhash:{bits:016x}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_crlf_and_nfc_share_an_alias() {
        let raw = snapshot_equivalent_ids(&PasteboardSnapshot {
            text: Some("a\r\nb".into()),
            ..Default::default()
        });
        let lf = snapshot_equivalent_ids(&PasteboardSnapshot {
            text: Some("a\nb".into()),
            ..Default::default()
        });
        assert!(raw.iter().any(|id| lf.contains(id)));
    }

    #[test]
    fn ledger_rejects_equivalent_text_and_accepts_new_text() {
        let mut ledger = ContentLedger::default();
        assert!(!ledger.observe(text_equivalent_ids("hello\r\n")));
        assert!(ledger.observe(text_equivalent_ids("hello\n")));
        assert!(ledger.observe(text_equivalent_ids("hello")));
        assert!(!ledger.observe(text_equivalent_ids("other")));
    }

    #[test]
    fn utf8_plain_text_item_shares_the_string_alias() {
        let from_string = snapshot_equivalent_ids(&PasteboardSnapshot {
            text: Some("same card".into()),
            ..Default::default()
        });
        let from_item = snapshot_equivalent_ids(&PasteboardSnapshot {
            items: vec![unidesk_platform::PasteboardItem {
                representations: vec![unidesk_platform::PasteboardRepresentation {
                    type_name: "public.utf8-plain-text".into(),
                    data: b"same card\n".to_vec(),
                }],
            }],
            ..Default::default()
        });
        assert!(from_string.iter().any(|id| from_item.contains(id)));
    }
}
