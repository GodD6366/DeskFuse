//! Windows Raw Input 旁路。低级钩子继续提供像素差；这里只累计设备计数。
//! 注册失败不让捕获线程退出，调用方保持绝对坐标。
#![allow(unsafe_code)]

use deskfuse_core::{accept_raw_delta, relative_delta_ok, set_relative_motion_capture_ready};
use std::{cell::Cell, ffi::c_void, mem::size_of, ptr::null_mut};
use windows_sys::Win32::{
    Foundation::{ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Input::{
            GetRawInputData, MOUSE_MOVE_ABSOLUTE, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER,
            RID_INPUT, RIDEV_INPUTSINK, RIM_TYPEMOUSE, RegisterRawInputDevices,
        },
        WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, HWND_MESSAGE, RegisterClassExW,
            WNDCLASSEXW,
        },
    },
};

thread_local! {
    static PENDING: Cell<(i64, i64, bool)> = const { Cell::new((0, 0, false)) };
}

/// 消息专用窗口。丢掉时解除注册目标并清除采集就绪。
pub(crate) struct RawMouseWindow {
    hwnd: HWND,
}

impl RawMouseWindow {
    /// `RegisterRawInputDevices` 成功才返回。失败时采集就绪保持假。
    pub(crate) fn install() -> Option<Self> {
        unsafe {
            let instance = GetModuleHandleW(std::ptr::null());
            if instance.is_null() {
                return None;
            }
            let class_name = wide_class_name();
            let class = WNDCLASSEXW {
                cbSize: size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(raw_wnd_proc),
                hInstance: instance,
                lpszClassName: class_name.as_ptr(),
                ..WNDCLASSEXW::default()
            };
            if RegisterClassExW(&class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
                return None;
            }
            let hwnd = CreateWindowExW(
                0,
                class_name.as_ptr(),
                class_name.as_ptr(),
                0,
                0,
                0,
                0,
                0,
                HWND_MESSAGE,
                null_mut(),
                instance,
                std::ptr::null(),
            );
            if hwnd.is_null() {
                return None;
            }
            let device = RAWINPUTDEVICE {
                usUsagePage: 0x01,
                usUsage: 0x02,
                dwFlags: RIDEV_INPUTSINK,
                hwndTarget: hwnd,
            };
            if RegisterRawInputDevices(&device, 1, size_of::<RAWINPUTDEVICE>() as u32) == 0 {
                DestroyWindow(hwnd);
                return None;
            }
            set_relative_motion_capture_ready(true);
            Some(Self { hwnd })
        }
    }
}

impl Drop for RawMouseWindow {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.hwnd);
        }
        clear_pending();
        set_relative_motion_capture_ready(false);
    }
}

/// 在 `TranslateMessage` 之前吃掉 `WM_INPUT`。句柄只在这条消息处理期间有效。
pub(crate) fn ingest(lparam: LPARAM) {
    let handle = lparam as *mut c_void;
    if handle.is_null() {
        return;
    }
    unsafe {
        let mut size = 0_u32;
        let header = size_of::<RAWINPUTHEADER>() as u32;
        let queried = GetRawInputData(handle, RID_INPUT, null_mut(), &mut size, header);
        if queried == u32::MAX || size == 0 || size > size_of::<RAWINPUT>() as u32 {
            return;
        }
        let mut raw = RAWINPUT::default();
        let copied = GetRawInputData(
            handle,
            RID_INPUT,
            &mut raw as *mut RAWINPUT as *mut c_void,
            &mut size,
            header,
        );
        if copied == u32::MAX || copied == 0 || raw.header.dwType != RIM_TYPEMOUSE {
            return;
        }
        let mouse = raw.data.mouse;
        let absolute = mouse.usFlags & MOUSE_MOVE_ABSOLUTE != 0;
        if let Some((dx, dy)) = accept_raw_delta(absolute, mouse.lLastX, mouse.lLastY) {
            note(dx, dy);
        }
    }
}

/// 取走自上次鼠标移动以来的设备计数。没有样本返回 `None`。
/// 和超过 32768 时丢掉整段累计，避免发出超限包。`Some((0, 0))` 只表示正负抵消。
#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
pub(crate) fn take_delta() -> Option<(i32, i32)> {
    PENDING.with(|cell| {
        let (x, y, has) = cell.replace((0, 0, false));
        if !has || !relative_delta_ok(x as f64) || !relative_delta_ok(y as f64) {
            return None;
        }
        Some((x as i32, y as i32))
    })
}

fn note(dx: i32, dy: i32) {
    PENDING.with(|cell| {
        let (x, y, _) = cell.get();
        cell.set((
            x.saturating_add(i64::from(dx)),
            y.saturating_add(i64::from(dy)),
            true,
        ));
    });
}

fn clear_pending() {
    PENDING.with(|cell| cell.set((0, 0, false)));
}

fn wide_class_name() -> Vec<u16> {
    "DeskFuseRawMouse"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect()
}

unsafe extern "system" fn raw_wnd_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}
