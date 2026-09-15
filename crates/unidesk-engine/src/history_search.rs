//! Search policy retained from the previous client; content remains in Rust.
pub fn clipboard_kind(item: &crate::model::ClipboardHistoryItem) -> &'static str {
    if !item.file_paths.is_empty() {
        "文件"
    } else if item.image_png.is_some()
        || item.mime.contains("image")
        || item.preview.contains("图像")
    {
        "图片"
    } else if item.mime.contains("text") {
        "文字"
    } else {
        "其他格式"
    }
}

pub fn clipboard_matches(
    item: &crate::model::ClipboardHistoryItem,
    query: &str,
    filter: usize,
) -> bool {
    let kind = clipboard_kind(item);
    let type_matches = match filter {
        1 => kind == "文字",
        2 => kind == "图片",
        3 => kind == "文件",
        _ => true,
    };
    let query = query.trim().to_lowercase();
    type_matches
        && (query.is_empty()
            || [&item.preview, &item.content, &item.source]
                .iter()
                .any(|text| text.to_lowercase().contains(&query)))
}

#[cfg(test)]
mod workspace_tests {
    use super::*;
    #[test]
    fn clipboard_search_uses_full_text_source_and_type() {
        let mut item = crate::model::ClipboardHistoryItem {
            id: 1,
            copied_at_ms: None,
            mime: "text/plain".into(),
            length: 20,
            preview: "摘要".into(),
            content: "完整正文还有 Liquid Glass".into(),
            content_truncated: false,
            source: "MacBook Pro".into(),
            image_png: None,
            file_paths: vec![],
        };
        assert!(clipboard_matches(&item, " liquid glass ", 1));
        assert!(clipboard_matches(&item, "macbook", 0));
        assert!(!clipboard_matches(&item, "摘要", 3));
        item.file_paths.push("/sample.txt".into());
        assert_eq!(clipboard_kind(&item), "文件");
        assert!(clipboard_matches(&item, "摘要", 3));
        assert!(!clipboard_matches(&item, "摘要", 1));
    }
}
