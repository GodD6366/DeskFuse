//! Versioned, lossless transport for native pasteboard items. There is no
//! content-type whitelist. File references use the separate file transfer path.
use bincode::Options;
use flate2::{Compression, read::ZlibDecoder, write::ZlibEncoder};
use std::io::{Read, Write};
use unidesk_platform::PasteboardItem;

pub const MIME: &str = "application/x-unidesk-pasteboard-v1";
const MAX_DECODED_BYTES: usize = 256 * 1024 * 1024;

fn codec() -> impl Options {
    bincode::DefaultOptions::new()
        .with_fixint_encoding()
        .with_limit(MAX_DECODED_BYTES as u64)
        .reject_trailing_bytes()
}

fn validate(items: &[PasteboardItem]) -> Result<(), String> {
    if items.is_empty() || items.len() > 512 {
        return Err("剪贴板项目数量无效".into());
    }
    let mut size = 0usize;
    for item in items {
        if item.representations.is_empty() || item.representations.len() > 512 {
            return Err("剪贴板格式数量无效".into());
        }
        let mut names = std::collections::BTreeSet::new();
        for rep in &item.representations {
            if rep.type_name.is_empty()
                || rep.type_name.len() > 4096
                || !names.insert(&rep.type_name)
            {
                return Err("剪贴板格式标识无效".into());
            }
            // These are host-local references/bookkeeping, never transferable
            // content. File paths must be replaced by verified received files.
            if matches!(
                rep.type_name.as_str(),
                "public.file-url"
                    | "NSFilenamesPboardType"
                    | "com.apple.pasteboard.promised-file-url"
                    | "com.apple.pasteboard.promised-file-content-type"
                    | "NSFilesPromisePboardType"
                    | "com.unidesk.clipboard-origin"
                    | "com.apple.is-remote-clipboard"
            ) {
                return Err("文件引用必须通过文件传输同步".into());
            }
            size = size
                .saturating_add(rep.type_name.len())
                .saturating_add(rep.data.len());
            if size > MAX_DECODED_BYTES {
                return Err("剪贴板内容超过 256 MiB，请使用文件传输".into());
            }
        }
    }
    Ok(())
}

pub fn encode(items: &[PasteboardItem]) -> Result<Vec<u8>, String> {
    validate(items)?;
    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    codec()
        .serialize_into(&mut encoder, items)
        .map_err(|e| e.to_string())?;
    encoder.flush().map_err(|e| e.to_string())?;
    encoder.finish().map_err(|e| e.to_string())
}

pub fn decode(data: &[u8]) -> Result<Vec<PasteboardItem>, String> {
    // Bound decompression before bincode allocates any nested vectors.
    let mut bytes = Vec::new();
    ZlibDecoder::new(data)
        .take(MAX_DECODED_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_DECODED_BYTES {
        return Err("剪贴板内容超过 256 MiB".into());
    }
    let items: Vec<PasteboardItem> = codec().deserialize(&bytes).map_err(|e| e.to_string())?;
    validate(&items)?;
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;
    use unidesk_platform::PasteboardRepresentation;

    #[test]
    fn all_native_formats_and_multiple_items_round_trip_losslessly() {
        let items = vec![
            PasteboardItem {
                representations: [
                    ("public.tiff", b"II\x2a\0image".as_slice()),
                    ("public.rtf", b"{\\rtf1\\b bold}"),
                    ("public.html", b"<b>bold</b>"),
                    ("com.example.editor.private-format", &[0, 1, 255, 0]),
                    ("public.pdf", b"%PDF-1.7"),
                ]
                .into_iter()
                .map(|(name, data)| PasteboardRepresentation {
                    type_name: name.into(),
                    data: data.into(),
                })
                .collect(),
            },
            PasteboardItem {
                representations: vec![PasteboardRepresentation {
                    type_name: "public.utf8-plain-text".into(),
                    data: b"second item".to_vec(),
                }],
            },
        ];
        assert_eq!(decode(&encode(&items).unwrap()).unwrap(), items);
    }

    #[test]
    fn invalid_bundle_does_not_become_a_pasteboard_write() {
        assert!(decode(b"not a compressed bundle").is_err());
        let items = vec![PasteboardItem {
            representations: vec![PasteboardRepresentation {
                type_name: "public.file-url".into(),
                data: b"file:///private/local-file".to_vec(),
            }],
        }];
        assert!(encode(&items).is_err());
    }
}
