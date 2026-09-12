use unidesk_platform::{MemoryPasteboard, Pasteboard, PasteboardItem, PasteboardRepresentation};

#[test]
fn file_references_replace_text_instead_of_becoming_path_text() {
    let clipboard = MemoryPasteboard::new();
    clipboard.write_text("path text").unwrap();
    let before = clipboard.change_count().unwrap();
    clipboard
        .write_file_urls(&["/fixture/file.txt".into()])
        .unwrap();
    let snapshot = clipboard.read().unwrap();
    assert!(snapshot.change_count > before);
    assert!(snapshot.text.is_none());
    assert_eq!(snapshot.file_urls, ["/fixture/file.txt"]);
}
#[test]
fn new_local_copy_clears_remote_origin_and_stale_file_offer() {
    let clipboard = MemoryPasteboard::new();
    clipboard.clear_for_remote_file("transfer-id").unwrap();
    clipboard.write_remote_origin_marker("digest").unwrap();
    clipboard.write_text("new copy").unwrap();
    assert_eq!(clipboard.read_remote_origin_marker().unwrap(), None);
    let snapshot = clipboard.read().unwrap();
    assert!(snapshot.file_urls.is_empty());
    assert_eq!(snapshot.text.as_deref(), Some("new copy"));
}
#[test]
fn native_representation_serialization_shape_remains_stable() {
    let clipboard = MemoryPasteboard::new();
    let item = PasteboardItem {
        representations: vec![PasteboardRepresentation {
            type_name: "public.rtf".into(),
            data: b"{\\rtf1}".to_vec(),
        }],
    };
    clipboard.write_items(std::slice::from_ref(&item)).unwrap();
    assert_eq!(clipboard.read().unwrap().items, vec![item]);
}
