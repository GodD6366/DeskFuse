//! Read the kernel hostname without spawning a process or waiting on pipe EOF.
#![allow(unsafe_code)]

pub fn hostname() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        use std::ffi::{c_char, c_int};
        unsafe extern "C" {
            fn gethostname(name: *mut c_char, len: usize) -> c_int;
        }
        let mut bytes = [0u8; 256];
        // SAFETY: the writable buffer is valid for the complete supplied length.
        if unsafe { gethostname(bytes.as_mut_ptr().cast(), bytes.len()) } != 0 {
            return None;
        }
        let end = bytes.iter().position(|byte| *byte == 0)?;
        let name = std::str::from_utf8(&bytes[..end]).ok()?.trim();
        (!name.is_empty()).then(|| name.to_owned())
    }
    #[cfg(not(target_os = "macos"))]
    None
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    #[test]
    fn reads_kernel_hostname_without_a_child_process() {
        let name = super::hostname().expect("kernel hostname");
        assert!(!name.is_empty());
        assert!(!name.contains('\0'));
        assert_eq!(Some(name), super::hostname());
    }
}
