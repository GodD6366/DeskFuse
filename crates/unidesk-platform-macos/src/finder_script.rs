//! Execute a fixed, read-only AppleScript in-process so automation consent is
//! attributed to uniDesk. OSAKit uses a private language instance on this worker;
//! no scripting objects cross threads and no process is spawned for polling.
#![allow(unsafe_code)]

use objc2::{
    class, msg_send,
    rc::{Retained, autoreleasepool},
    runtime::AnyObject,
};
use objc2_foundation::{NSAppleEventDescriptor, NSArray, NSDictionary, NSNumber, NSString, NSURL};
use std::{cell::RefCell, path::PathBuf};

#[link(name = "OSAKit", kind = "framework")]
unsafe extern "C" {
    static OSAScriptErrorNumber: &'static NSString;
}

const SOURCE: &str = include_str!("finder_destination.applescript");
type ScriptErrors = NSDictionary<NSString, AnyObject>;

thread_local! {
    static SCRIPT: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
}

#[derive(PartialEq, Eq)]
pub(super) struct FinderLocation {
    pub window_id: i32,
    pub path: PathBuf,
}

fn script_error(code: i32) -> String {
    let message = match code {
        -1743 | -1744 => {
            "请在系统设置 → 隐私与安全性 → 自动化中允许 uniDesk 控制 Finder，然后重试粘贴"
        }
        -1712 => "Finder 目录查询超时，请等待 Finder 响应后重试",
        -2702 => "请先打开要粘贴到的 Finder 文件夹",
        -1700 | -1728 => "Finder 当前位置无法转换为实际文件夹，请打开目标文件夹后粘贴",
        -128 => "Finder 自动化授权或目录查询已取消",
        _ => "AppleScript 无法读取 Finder 粘贴目录",
    };
    format!("{message}（AppleScript {code}）。")
}

fn execution_error(errors: Option<&ScriptErrors>) -> String {
    // SAFETY: OSAKit exports this immutable NSString for the process lifetime.
    let key = unsafe { OSAScriptErrorNumber };
    let code = errors
        .and_then(|errors| errors.objectForKey(key))
        .and_then(|value| {
            value
                .downcast_ref::<NSNumber>()
                .map(|number| number.intValue())
        })
        .unwrap_or(-2700);
    script_error(code)
}

pub(super) fn location(desktop: bool) -> Result<FinderLocation, String> {
    autoreleasepool(|_| {
        SCRIPT.with(|slot| {
            let mut slot = slot.borrow_mut();
            if slot.is_none() {
                // SAFETY: These are documented OSAKit constructors/selectors.
                // The language and private instance remain on this native worker.
                // Nil returns are represented with Option, not assumed non-null.
                let script: Retained<AnyObject> = unsafe {
                    let language: Option<Retained<AnyObject>> = msg_send![
                        class!(OSALanguage), languageForName: &*NSString::from_str("AppleScript")
                    ];
                    let language = language.ok_or("系统未提供 AppleScript 语言组件。")?;
                    let thread_safe: bool = msg_send![&*language, isThreadSafe];
                    if !thread_safe {
                        return Err("系统 AppleScript 组件不支持后台目录查询。".into());
                    }
                    let instance: Option<Retained<AnyObject>> = msg_send![
                        class!(OSALanguageInstance), languageInstanceWithLanguage: &*language
                    ];
                    let instance = instance.ok_or("无法创建 Finder 查询组件。")?;
                    let allocated: objc2::rc::Allocated<AnyObject> =
                        msg_send![class!(OSAScript), alloc];
                    let script: Option<Retained<AnyObject>> = msg_send![allocated,
                        initWithSource: &*NSString::from_str(SOURCE),
                        fromURL: None::<&NSURL>,
                        languageInstance: &*instance,
                        usingStorageOptions: 0_usize
                    ];
                    script.ok_or("无法初始化 Finder 目录查询。")?
                };
                let mut errors: Option<Retained<ScriptErrors>> = None;
                // SAFETY: OSAKit returns a dictionary of NSString keys/id values
                // through an autoreleasing out parameter; objc2 retains the result.
                let compiled: bool =
                    unsafe { msg_send![&*script, compileAndReturnError: &mut errors] };
                if !compiled {
                    return Err(execution_error(errors.as_deref()));
                }
                *slot = Some(script);
            }
            let script = slot.as_ref().ok_or("Finder 查询组件不可用。")?;
            let arguments = NSArray::from_retained_slice(&[NSNumber::numberWithBool(desktop)]);
            let mut errors: Option<Retained<ScriptErrors>> = None;
            // SAFETY: Handler name and argument types match our embedded script.
            // Execution stays serialized in the thread-local instance. Treat nil
            // as failure and never log raw script results or error dictionaries.
            let result: Option<Retained<NSAppleEventDescriptor>> = unsafe {
                msg_send![&**script,
                    executeHandlerWithName: &*NSString::from_str("readDestination"),
                    arguments: &*arguments,
                    error: &mut errors
                ]
            };
            let result = result.ok_or_else(|| execution_error(errors.as_deref()))?;
            if result.numberOfItems() != 3 {
                return Err("Finder 目录查询返回了无效结果。".into());
            }
            let code = result
                .descriptorAtIndex(1)
                .map(|item| item.int32Value())
                .unwrap_or(-2700);
            if code != 0 {
                return Err(script_error(code));
            }
            let window_id = result
                .descriptorAtIndex(2)
                .map(|item| item.int32Value())
                .ok_or("Finder 未返回窗口身份。")?;
            let path = result
                .descriptorAtIndex(3)
                .and_then(|item| item.stringValue())
                .map(|value| PathBuf::from(value.to_string()))
                .filter(|path| path.is_absolute())
                .ok_or("Finder 未返回绝对目录路径。")?;
            Ok(FinderLocation { window_id, path })
        })
    })
}
