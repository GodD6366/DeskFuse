//! macOS pasteboard (clipboard) boundary.
//!
//! The native implementation reads and writes the general pasteboard through
//! AppKit. Clipboard bodies are returned as opaque bytes and never appear in
//! diagnostics, and a deterministic in-memory implementation is provided for
//! tests.

use std::collections::BTreeSet;

use objc2::{ClassType, rc::Retained, runtime::AnyObject};
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSString, NSURL};

use crate::{PlatformError, PlatformResult};

pub use unidesk_platform::{
    MemoryPasteboard, Pasteboard, PasteboardItem, PasteboardRepresentation, PasteboardSnapshot,
    REMOTE_ORIGIN_MARKER_TYPE, UNIVERSAL_CLIPBOARD_TYPE,
};

fn is_remote_bookkeeping(type_name: &str) -> bool {
    matches!(
        type_name,
        REMOTE_ORIGIN_MARKER_TYPE | UNIVERSAL_CLIPBOARD_TYPE
    )
}

/// Native macOS general pasteboard.
#[derive(Clone, Copy, Debug, Default)]
pub struct MacOSPasteboard;

impl MacOSPasteboard {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

/// File references whose producer may still need to materialize the bytes.
const PROMISED_FILE_URL_TYPE: &str = "com.apple.pasteboard.promised-file-url";
const FILE_PROMISE_CONTENT_TYPE: &str = "com.apple.pasteboard.promised-file-content-type";
/// Legacy file promises contain file extensions, never source paths.
const FILES_PROMISE_TYPE: &str = "NSFilesPromisePboardType";

/// Inserts the filesystem path resolved from a `file://` URL string.
fn insert_file_url_path(paths: &mut BTreeSet<String>, url_string: &str) {
    let url = NSString::from_str(url_string);
    if let Some(path) = NSURL::URLWithString(&url)
        .filter(|url| url.isFileURL())
        .and_then(|url| url.path())
        .filter(|path| std::path::Path::new(&path.to_string()).is_absolute())
    {
        paths.insert(path.to_string());
    }
}

#[allow(unsafe_code, deprecated)]
fn read_file_paths(pasteboard: &objc2_app_kit::NSPasteboard) -> PlatformResult<Vec<String>> {
    let file_url_type = unsafe { objc2_app_kit::NSPasteboardTypeFileURL };
    let legacy_filenames_type = unsafe { objc2_app_kit::NSFilenamesPboardType };
    let promised_url_type = NSString::from_str(PROMISED_FILE_URL_TYPE);
    // A pasteboard is file-like when it carries any file-reference or
    // file-promise representation. Promises are included so an
    // unmaterialized cloud/network copy never falls through to its textual
    // filename, which would otherwise be pasted as a `.textClipping` on the
    // receiving Mac.
    let has_file_like_content = pasteboard.types().is_some_and(|types| {
        (0..types.count()).any(|index| {
            let value = types.objectAtIndex(index).to_string();
            value == file_url_type.to_string()
                || value == legacy_filenames_type.to_string()
                || value == PROMISED_FILE_URL_TYPE
                || value == FILE_PROMISE_CONTENT_TYPE
                || value == FILES_PROMISE_TYPE
        })
    });

    let mut paths = BTreeSet::new();

    // Modern Finder pasteboards normally expose one public.file-url string
    // per NSPasteboardItem.
    if let Some(items) = pasteboard.pasteboardItems() {
        for index in 0..items.count() {
            let item = items.objectAtIndex(index);
            // Promises are item-scoped too: a pasteboard-level read only
            // returns the first item's value in a multi-file copy.
            for representation in [file_url_type, &promised_url_type] {
                if let Some(value) = item.stringForType(representation) {
                    insert_file_url_path(&mut paths, &value.to_string());
                }
            }
        }
    }

    // Some applications only publish native NSURL objects even though the
    // pasteboard declares public.file-url.
    let classes = NSArray::from_slice(&[NSURL::class()]);
    let file_only: Retained<AnyObject> = NSNumber::numberWithBool(true).into();
    let options: Retained<NSDictionary<NSString, AnyObject>> = NSDictionary::from_slices(
        &[unsafe { objc2_app_kit::NSPasteboardURLReadingFileURLsOnlyKey }],
        &[&file_only],
    );
    // SAFETY: NSURL is a valid NSPasteboardReading class and the options
    // dictionary contains Apple's documented file-URLs-only boolean.
    if let Some(objects) =
        unsafe { pasteboard.readObjectsForClasses_options(&classes, Some(&options)) }
    {
        for index in 0..objects.count() {
            let object = objects.objectAtIndex(index);
            if let Ok(url) = object.downcast::<NSURL>()
                && url.isFileURL()
                && let Some(path) = url.path()
            {
                paths.insert(path.to_string());
            }
        }
    }

    // Legacy Finder integrations still use NSFilenamesPboardType, whose
    // property list is an NSArray of filesystem path strings.
    if let Some(value) = pasteboard.propertyListForType(legacy_filenames_type)
        && let Some(array) = value.downcast_ref::<NSArray>()
    {
        for index in 0..array.count() {
            let object = array.objectAtIndex(index);
            if let Ok(path) = object.downcast::<NSString>()
                && std::path::Path::new(&path.to_string()).is_absolute()
            {
                paths.insert(path.to_string());
            }
        }
    }

    // Modern file promise: one file URL string, or an array of URL strings for
    // a multi-file cut/copy.
    if let Some(value) = pasteboard.stringForType(&promised_url_type) {
        insert_file_url_path(&mut paths, &value.to_string());
    }
    if let Some(value) = pasteboard.propertyListForType(&promised_url_type) {
        if let Some(url) = value.downcast_ref::<NSString>() {
            insert_file_url_path(&mut paths, &url.to_string());
        } else if let Some(array) = value.downcast_ref::<NSArray>() {
            for index in 0..array.count() {
                let object = array.objectAtIndex(index);
                if let Ok(url) = object.downcast::<NSString>() {
                    insert_file_url_path(&mut paths, &url.to_string());
                }
            }
        }
    }

    // A file-like pasteboard must never fall through to its textual filename
    // representation. Finder would materialize that text as .textClipping on
    // the receiving Mac.
    if has_file_like_content && paths.is_empty() {
        return Err(PlatformError::PasteboardFileUnavailable);
    }
    Ok(paths.into_iter().collect())
}

fn read_items(pasteboard: &objc2_app_kit::NSPasteboard) -> PlatformResult<Vec<PasteboardItem>> {
    let mut result = Vec::new();
    if let Some(items) = pasteboard.pasteboardItems() {
        for index in 0..items.count() {
            let item = items.objectAtIndex(index);
            let mut representations = Vec::new();
            let types = item.types();
            for type_index in 0..types.count() {
                let kind = types.objectAtIndex(type_index);
                let type_name = kind.to_string();
                if is_remote_bookkeeping(&type_name) {
                    continue;
                }
                // Materialize promised data while the original owner is alive.
                // Some OS metadata types declare no data; they are not content.
                if let Some(data) = item.dataForType(&kind) {
                    representations.push(PasteboardRepresentation {
                        type_name,
                        data: data.to_vec(),
                    });
                }
            }
            representations.sort_by(|a, b| a.type_name.cmp(&b.type_name));
            if !representations.is_empty() {
                result.push(PasteboardItem { representations });
            }
        }
    }
    Ok(result)
}

fn write_items(
    pasteboard: &objc2_app_kit::NSPasteboard,
    items: &[PasteboardItem],
) -> PlatformResult<()> {
    use objc2::runtime::ProtocolObject;
    // Construct everything before replacing the user's current clipboard.
    if items.is_empty() || items.iter().any(|item| item.representations.is_empty()) {
        return Err(PlatformError::PasteboardWriteFailed);
    }
    let mut objects = Vec::with_capacity(items.len());
    for item in items {
        let native = objc2_app_kit::NSPasteboardItem::new();
        for representation in &item.representations {
            if is_remote_bookkeeping(&representation.type_name) {
                continue;
            }
            let kind = NSString::from_str(&representation.type_name);
            let data = objc2_foundation::NSData::with_bytes(&representation.data);
            if !native.setData_forType(&data, &kind) {
                return Err(PlatformError::PasteboardWriteFailed);
            }
        }
        objects
            .push(ProtocolObject::<dyn objc2_app_kit::NSPasteboardWriting>::from_retained(native));
    }
    pasteboard.prepareForNewContentsWithOptions(
        objc2_app_kit::NSPasteboardContentsOptions::CurrentHostOnly,
    );
    if pasteboard.writeObjects(&NSArray::from_retained_slice(&objects)) {
        Ok(())
    } else {
        Err(PlatformError::PasteboardWriteFailed)
    }
}

impl Pasteboard for MacOSPasteboard {
    fn change_count(&self) -> PlatformResult<i64> {
        let pasteboard = objc2_app_kit::NSPasteboard::generalPasteboard();
        Ok(pasteboard.changeCount() as i64)
    }

    #[allow(unsafe_code)]
    fn read(&self) -> PlatformResult<PasteboardSnapshot> {
        let pasteboard = objc2_app_kit::NSPasteboard::generalPasteboard();
        let change_count = pasteboard.changeCount() as i64;
        // SAFETY: the pasteboard type constants are immutable, well-known
        // extern statics; passing them by reference is the documented ABI.
        let text = unsafe { pasteboard.stringForType(objc2_app_kit::NSPasteboardTypeString) }
            .map(|string| string.to_string());
        let png = unsafe { pasteboard.dataForType(objc2_app_kit::NSPasteboardTypePNG) }
            .map(|data| data.to_vec());
        let file_urls = read_file_paths(&pasteboard)?;
        let items = if file_urls.is_empty() {
            read_items(&pasteboard)?
        } else {
            Vec::new()
        };
        // Do not combine formats from two different copy operations if the
        // owner changed while materializing a large or lazy representation.
        if pasteboard.changeCount() as i64 != change_count {
            return Err(PlatformError::PasteboardReadFailed);
        }
        Ok(PasteboardSnapshot {
            change_count,
            text,
            png,
            file_urls,
            items,
        })
    }

    #[allow(unsafe_code)]
    fn write_text(&self, text: &str) -> PlatformResult<()> {
        let pasteboard = objc2_app_kit::NSPasteboard::generalPasteboard();
        let string = objc2_foundation::NSString::from_str(text);
        // Remote content must not enter Apple's Universal Clipboard/Handoff
        // path; uniDesk already delivered it to the intended trusted peers.
        let _ = pasteboard.prepareForNewContentsWithOptions(
            objc2_app_kit::NSPasteboardContentsOptions::CurrentHostOnly,
        );
        // SAFETY: immutable, well-known pasteboard type constant.
        let ok =
            unsafe { pasteboard.setString_forType(&string, objc2_app_kit::NSPasteboardTypeString) };
        if ok {
            Ok(())
        } else {
            Err(PlatformError::PasteboardWriteFailed)
        }
    }

    #[allow(unsafe_code)]
    fn write_png(&self, bytes: &[u8]) -> PlatformResult<()> {
        let pasteboard = objc2_app_kit::NSPasteboard::generalPasteboard();
        let data = objc2_foundation::NSData::with_bytes(bytes);
        // See `write_text`: receiving a peer update must not re-enter Handoff.
        let _ = pasteboard.prepareForNewContentsWithOptions(
            objc2_app_kit::NSPasteboardContentsOptions::CurrentHostOnly,
        );
        // SAFETY: immutable, well-known pasteboard type constant.
        let ok =
            unsafe { pasteboard.setData_forType(Some(&data), objc2_app_kit::NSPasteboardTypePNG) };
        if ok {
            Ok(())
        } else {
            Err(PlatformError::PasteboardWriteFailed)
        }
    }

    fn write_file_urls(&self, paths: &[String]) -> PlatformResult<()> {
        use objc2::runtime::ProtocolObject;

        let pasteboard = objc2_app_kit::NSPasteboard::generalPasteboard();
        let objects = paths
            .iter()
            .map(|path| {
                let path = objc2_foundation::NSString::from_str(path);
                let url = objc2_foundation::NSURL::fileURLWithPath(&path);
                ProtocolObject::<dyn objc2_app_kit::NSPasteboardWriting>::from_retained(url)
            })
            .collect::<Vec<_>>();
        let objects = objc2_foundation::NSArray::from_retained_slice(&objects);
        // Files may be large; never let a received file reference re-enter
        // Universal Clipboard while uniDesk is still suppressing its echo.
        let _ = pasteboard.prepareForNewContentsWithOptions(
            objc2_app_kit::NSPasteboardContentsOptions::CurrentHostOnly,
        );
        if pasteboard.writeObjects(&objects) {
            Ok(())
        } else {
            Err(PlatformError::PasteboardWriteFailed)
        }
    }

    fn clear_for_remote_file(&self, marker: &str) -> PlatformResult<()> {
        objc2_app_kit::NSPasteboard::generalPasteboard().prepareForNewContentsWithOptions(
            objc2_app_kit::NSPasteboardContentsOptions::CurrentHostOnly,
        );
        self.write_remote_origin_marker(marker)
    }

    fn write_items(&self, items: &[PasteboardItem]) -> PlatformResult<()> {
        write_items(&objc2_app_kit::NSPasteboard::generalPasteboard(), items)
    }

    fn read_remote_origin_marker(&self) -> PlatformResult<Option<String>> {
        let pasteboard = objc2_app_kit::NSPasteboard::generalPasteboard();
        let marker_type = NSString::from_str(REMOTE_ORIGIN_MARKER_TYPE);
        Ok(pasteboard
            .dataForType(&marker_type)
            .and_then(|data| String::from_utf8(data.to_vec()).ok()))
    }

    fn contains_type(&self, type_name: &str) -> PlatformResult<bool> {
        let pasteboard = objc2_app_kit::NSPasteboard::generalPasteboard();
        if pasteboard.types().is_some_and(|types| {
            (0..types.count()).any(|index| types.objectAtIndex(index).to_string() == type_name)
        }) {
            return Ok(true);
        }
        if let Some(items) = pasteboard.pasteboardItems() {
            for index in 0..items.count() {
                let types = items.objectAtIndex(index).types();
                if (0..types.count())
                    .any(|type_index| types.objectAtIndex(type_index).to_string() == type_name)
                {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn write_remote_origin_marker(&self, marker: &str) -> PlatformResult<()> {
        let pasteboard = objc2_app_kit::NSPasteboard::generalPasteboard();
        let marker_type = NSString::from_str(REMOTE_ORIGIN_MARKER_TYPE);
        let data = objc2_foundation::NSData::with_bytes(marker.as_bytes());
        if pasteboard.setData_forType(Some(&data), &marker_type) {
            Ok(())
        } else {
            Err(PlatformError::PasteboardWriteFailed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use objc2::runtime::ProtocolObject;

    #[test]
    fn native_tiff_and_custom_formats_are_preserved_without_png() {
        let source = objc2_app_kit::NSPasteboard::pasteboardWithUniqueName();
        let tiff = objc2_foundation::NSData::with_bytes(b"II\x2a\0test image");
        let custom = objc2_foundation::NSData::with_bytes(&[0, 255, 7, 0]);
        assert!(source.setData_forType(Some(&tiff), &NSString::from_str("public.tiff")));
        assert!(source.setData_forType(
            Some(&custom),
            &NSString::from_str("com.example.design.selection")
        ));
        let items = read_items(&source).expect("read all formats");
        assert_eq!(items.len(), 1);
        assert!(
            items[0]
                .representations
                .iter()
                .any(|r| r.type_name == "public.tiff" && r.data == tiff.to_vec())
        );
        assert!(
            !items[0]
                .representations
                .iter()
                .any(|r| r.type_name == "public.png")
        );
        let destination = objc2_app_kit::NSPasteboard::pasteboardWithUniqueName();
        write_items(&destination, &items).expect("write all formats");
        assert_eq!(read_items(&destination).expect("read remote"), items);
    }

    #[test]
    fn native_rich_text_and_multiple_item_boundaries_survive() {
        let items = vec![
            PasteboardItem {
                representations: vec![
                    PasteboardRepresentation {
                        type_name: "public.html".into(),
                        data: b"<b>bold</b>".to_vec(),
                    },
                    PasteboardRepresentation {
                        type_name: "public.rtf".into(),
                        data: b"{\\rtf1\\b bold}".to_vec(),
                    },
                ],
            },
            PasteboardItem {
                representations: vec![PasteboardRepresentation {
                    type_name: "com.example.private".into(),
                    data: vec![0, 1, 255],
                }],
            },
        ];
        let pasteboard = objc2_app_kit::NSPasteboard::pasteboardWithUniqueName();
        write_items(&pasteboard, &items).expect("native write");
        let actual = read_items(&pasteboard).expect("native read");
        assert_eq!(actual.len(), 2);
        for (expected, actual) in items.iter().zip(&actual) {
            for representation in &expected.representations {
                assert!(actual.representations.contains(representation));
            }
        }
    }

    #[test]
    fn native_origin_bookkeeping_is_not_forwarded_as_content() {
        let pasteboard = objc2_app_kit::NSPasteboard::pasteboardWithUniqueName();
        assert!(pasteboard.setString_forType(
            &NSString::from_str("test"),
            &NSString::from_str("public.utf8-plain-text")
        ));
        assert!(pasteboard.setString_forType(
            &NSString::from_str("digest"),
            &NSString::from_str(REMOTE_ORIGIN_MARKER_TYPE)
        ));
        let items = read_items(&pasteboard).expect("native read");
        assert!(
            items
                .iter()
                .flat_map(|item| &item.representations)
                .all(|r| r.type_name != REMOTE_ORIGIN_MARKER_TYPE)
        );
    }

    #[test]
    fn memory_pasteboard_marker_is_private_and_local_writes_clear_it() {
        let pasteboard = MemoryPasteboard::new();
        pasteboard.write_text("from peer").expect("write text");
        pasteboard
            .write_remote_origin_marker("unidesk-clipboard-v1:digest")
            .expect("write marker");
        assert_eq!(
            pasteboard.read_remote_origin_marker().expect("read marker"),
            Some("unidesk-clipboard-v1:digest".to_owned())
        );

        pasteboard
            .write_text("local copy")
            .expect("write local text");
        assert_eq!(
            pasteboard.read_remote_origin_marker().expect("read marker"),
            None
        );
    }

    #[test]
    fn reads_native_file_url_objects_from_a_pasteboard() {
        let pasteboard = objc2_app_kit::NSPasteboard::pasteboardWithUniqueName();
        let path = NSString::from_str("/tmp/unidesk-native-file.png");
        let url = NSURL::fileURLWithPath(&path);
        let object = ProtocolObject::<dyn objc2_app_kit::NSPasteboardWriting>::from_retained(url);
        let objects = NSArray::from_retained_slice(&[object]);
        pasteboard.clearContents();
        assert!(pasteboard.writeObjects(&objects));

        assert_eq!(
            read_file_paths(&pasteboard).expect("file paths"),
            vec!["/tmp/unidesk-native-file.png".to_owned()]
        );
    }

    #[test]
    fn resolves_finder_style_file_reference_urls_to_real_paths() {
        let unique = format!("unidesk-finder-reference-{}.txt", std::process::id());
        let expected = std::env::temp_dir().join(unique);
        std::fs::write(&expected, b"finder reference").expect("create fixture");
        let pasteboard = objc2_app_kit::NSPasteboard::pasteboardWithUniqueName();
        let path = NSString::from_str(&expected.to_string_lossy());
        let file_url = NSURL::fileURLWithPath(&path);
        let reference_url = file_url.fileReferenceURL().expect("file reference URL");
        let object =
            ProtocolObject::<dyn objc2_app_kit::NSPasteboardWriting>::from_retained(reference_url);
        let objects = NSArray::from_retained_slice(&[object]);
        pasteboard.clearContents();
        assert!(pasteboard.writeObjects(&objects));

        assert_eq!(
            read_file_paths(&pasteboard).expect("resolved file paths"),
            vec![
                std::fs::canonicalize(&expected)
                    .expect("canonical fixture path")
                    .to_string_lossy()
                    .into_owned()
            ]
        );
        std::fs::remove_file(expected).expect("remove fixture");
    }

    #[test]
    #[allow(unsafe_code, deprecated)]
    fn reads_legacy_finder_filename_property_list() {
        let pasteboard = objc2_app_kit::NSPasteboard::pasteboardWithUniqueName();
        let path = NSString::from_str("/tmp/unidesk-legacy-file.png");
        let paths = NSArray::from_slice(&[&*path]);
        pasteboard.clearContents();
        // SAFETY: NSFilenamesPboardType requires an NSArray of NSString paths,
        // which is exactly what is provided here.
        assert!(unsafe {
            pasteboard.setPropertyList_forType(&paths, objc2_app_kit::NSFilenamesPboardType)
        });

        assert_eq!(
            read_file_paths(&pasteboard).expect("legacy file paths"),
            vec!["/tmp/unidesk-legacy-file.png".to_owned()]
        );
    }

    #[test]
    fn reads_modern_file_promise_urls() {
        let pasteboard = objc2_app_kit::NSPasteboard::pasteboardWithUniqueName();
        let promised_type = NSString::from_str(PROMISED_FILE_URL_TYPE);
        let url = NSString::from_str("file:///tmp/unidesk-promised-file.txt");
        pasteboard.clearContents();
        assert!(pasteboard.setString_forType(&url, &promised_type));

        assert_eq!(
            read_file_paths(&pasteboard).expect("promised file paths"),
            vec!["/tmp/unidesk-promised-file.txt".to_owned()]
        );
    }

    #[test]
    fn reads_every_promised_file_item() {
        let pasteboard = objc2_app_kit::NSPasteboard::pasteboardWithUniqueName();
        let promised_type = NSString::from_str(PROMISED_FILE_URL_TYPE);
        let objects = ["file:///tmp/first.txt", "file:///tmp/second.txt"].map(|url| {
            let item = objc2_app_kit::NSPasteboardItem::new();
            assert!(item.setString_forType(&NSString::from_str(url), &promised_type));
            ProtocolObject::<dyn objc2_app_kit::NSPasteboardWriting>::from_retained(item)
        });
        assert!(pasteboard.writeObjects(&NSArray::from_retained_slice(&objects)));
        assert_eq!(
            read_file_paths(&pasteboard).expect("all promised paths"),
            vec!["/tmp/first.txt", "/tmp/second.txt"]
        );
    }

    #[test]
    fn unresolved_file_promise_with_text_cannot_be_synced_as_text() {
        let pasteboard = objc2_app_kit::NSPasteboard::pasteboardWithUniqueName();
        assert!(pasteboard.setString_forType(
            &NSString::from_str("public.pdf"),
            &NSString::from_str("com.apple.pasteboard.promised-file-content-type"),
        ));
        assert!(pasteboard.setString_forType(
            &NSString::from_str("document.pdf"),
            &NSString::from_str("public.utf8-plain-text"),
        ));
        assert!(read_file_paths(&pasteboard).is_err());
    }

    #[test]
    fn non_file_url_is_not_interpreted_as_a_local_file() {
        let pasteboard = objc2_app_kit::NSPasteboard::pasteboardWithUniqueName();
        assert!(pasteboard.setString_forType(
            &NSString::from_str("https://example.com/tmp/document.pdf"),
            &NSString::from_str(PROMISED_FILE_URL_TYPE),
        ));
        assert!(read_file_paths(&pasteboard).is_err());
    }

    #[test]
    #[allow(unsafe_code, deprecated)]
    fn legacy_file_promise_extensions_are_not_paths() {
        let pasteboard = objc2_app_kit::NSPasteboard::pasteboardWithUniqueName();
        let promise_type = NSString::from_str(FILES_PROMISE_TYPE);
        let path = NSString::from_str("pdf");
        let paths = NSArray::from_slice(&[&*path]);
        pasteboard.clearContents();
        // SAFETY: legacy file promises contain an NSArray of NSString file
        // extensions, not filesystem paths.
        assert!(unsafe { pasteboard.setPropertyList_forType(&paths, &promise_type) });

        assert!(read_file_paths(&pasteboard).is_err());
    }
}
