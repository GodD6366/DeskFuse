//! PCM-to-Roc bridge for the Windows microphone session.
//!
//! This module is opt-in until the signed `deskfuse-roc-microphone.dll` is
//! present beside the Windows service. It deliberately uses the existing
//! `MicrophoneSession` packet contract, so Windows peers stay compatible with
//! the macOS sender/receiver.

use crate::native::microphone::Capture;
use deskfuse_transport::microphone::MicrophoneSession;
use std::sync::Arc;

#[repr(C)]
struct RocSender;
#[repr(C)]
struct RocReceiver;

#[link(name = "deskfuse-roc-microphone")]
unsafe extern "C" {
    fn deskfuse_roc_sender_create() -> *mut RocSender;
    fn deskfuse_roc_sender_push_f32(
        sender: *mut RocSender,
        samples: *const f32,
        frames: usize,
    ) -> i32;
    fn deskfuse_roc_sender_pop(
        sender: *mut RocSender,
        interface_id: i32,
        bytes: *mut u8,
        capacity: usize,
    ) -> i32;
    fn deskfuse_roc_sender_feedback(sender: *mut RocSender, bytes: *const u8, length: usize)
        -> i32;
    fn deskfuse_roc_sender_destroy(sender: *mut RocSender);
    fn deskfuse_roc_sender_last_error() -> *const std::ffi::c_char;
    fn deskfuse_roc_receiver_create() -> *mut RocReceiver;
    fn deskfuse_roc_receiver_push(
        receiver: *mut RocReceiver,
        interface_id: i32,
        bytes: *const u8,
        length: usize,
    ) -> i32;
    fn deskfuse_roc_receiver_pop_f32(
        receiver: *mut RocReceiver,
        samples: *mut f32,
        frames: usize,
    ) -> i32;
    fn deskfuse_roc_receiver_pop_feedback(
        receiver: *mut RocReceiver,
        bytes: *mut u8,
        capacity: usize,
    ) -> i32;
    fn deskfuse_roc_receiver_destroy(receiver: *mut RocReceiver);
}

pub struct RocSessionPump {
    sender: *mut RocSender,
}
unsafe impl Send for RocSessionPump {}
unsafe impl Sync for RocSessionPump {}

impl RocSessionPump {
    pub fn new() -> Result<Self, String> {
        // SAFETY: the DLL owns the opaque sender and returns null on failure.
        let sender = unsafe { deskfuse_roc_sender_create() };
        if sender.is_null() {
            return Err(last_error());
        }
        Ok(Self { sender })
    }

    pub fn pump_capture(
        &self,
        capture: &Capture,
        session: &MicrophoneSession,
    ) -> Result<(), String> {
        while let Some(frame) = capture.try_frame().map_err(|e| e.to_string())? {
            let samples = frame
                .into_iter()
                .map(|sample| f32::from(sample) / f32::from(i16::MAX))
                .collect::<Vec<_>>();
            // SAFETY: samples live for the duration of the synchronous call.
            let result = unsafe {
                deskfuse_roc_sender_push_f32(self.sender, samples.as_ptr(), samples.len())
            };
            if result < 0 {
                return Err(last_error());
            }
            self.drain_packets(session)?;
        }
        self.apply_feedback(session)
    }

    fn drain_packets(&self, session: &MicrophoneSession) -> Result<(), String> {
        for interface_id in 1..=3 {
            for _ in 0..16 {
                let mut packet = vec![0u8; 4096];
                // SAFETY: packet is writable for the whole synchronous call.
                let length = unsafe {
                    deskfuse_roc_sender_pop(
                        self.sender,
                        interface_id,
                        packet.as_mut_ptr(),
                        packet.len(),
                    )
                };
                if length < 0 {
                    return Err(last_error());
                }
                if length == 0 {
                    break;
                }
                packet.truncate(length as usize);
                let mut framed = Vec::with_capacity(packet.len() + 1);
                framed.push(interface_id as u8);
                framed.extend_from_slice(&packet);
                session.send(framed)?;
            }
        }
        Ok(())
    }

    fn apply_feedback(&self, session: &MicrophoneSession) -> Result<(), String> {
        for _ in 0..24 {
            let Some(packet) = session.receive()? else {
                break;
            };
            if packet.first() == Some(&3) {
                // SAFETY: packet remains alive for the synchronous call.
                let result = unsafe {
                    deskfuse_roc_sender_feedback(
                        self.sender,
                        packet[1..].as_ptr(),
                        packet.len() - 1,
                    )
                };
                if result < 0 {
                    return Err(last_error());
                }
            }
        }
        Ok(())
    }
}

impl Drop for RocSessionPump {
    fn drop(&mut self) {
        // SAFETY: sender was created by the matching DLL function exactly once.
        unsafe { deskfuse_roc_sender_destroy(self.sender) }
    }
}

fn last_error() -> String {
    // SAFETY: the DLL returns a thread-local NUL-terminated error string.
    let pointer = unsafe { deskfuse_roc_sender_last_error() };
    if pointer.is_null() {
        return "Roc sender 初始化失败".into();
    }
    unsafe { std::ffi::CStr::from_ptr(pointer) }
        .to_string_lossy()
        .into_owned()
}

pub struct RocReceiverPump {
    receiver: *mut RocReceiver,
}
unsafe impl Send for RocReceiverPump {}
unsafe impl Sync for RocReceiverPump {}

impl RocReceiverPump {
    pub fn new() -> Result<Self, String> {
        // SAFETY: the DLL owns the opaque receiver and returns null on failure.
        let receiver = unsafe { deskfuse_roc_receiver_create() };
        if receiver.is_null() {
            return Err(last_error());
        }
        Ok(Self { receiver })
    }

    pub fn pump_session(&self, session: &MicrophoneSession) -> Result<Vec<f32>, String> {
        for _ in 0..24 {
            let Some(packet) = session.receive()? else {
                break;
            };
            let Some(&interface_id) = packet.first() else {
                continue;
            };
            if !(1..=3).contains(&interface_id) || packet.len() < 2 {
                return Err("麦克风 session 返回了无效的 Roc packet".into());
            }
            // SAFETY: packet remains alive during the synchronous decoder call.
            let result = unsafe {
                deskfuse_roc_receiver_push(
                    self.receiver,
                    i32::from(interface_id),
                    packet[1..].as_ptr(),
                    packet.len() - 1,
                )
            };
            if result < 0 {
                return Err(last_error());
            }
        }
        let mut samples = vec![0.0f32; 480];
        // SAFETY: samples is writable for the synchronous decoder call.
        let result = unsafe {
            deskfuse_roc_receiver_pop_f32(self.receiver, samples.as_mut_ptr(), samples.len())
        };
        if result < 0 {
            return Err(last_error());
        }
        if result == 0 {
            samples.truncate(0);
        }
        let mut feedback = vec![0u8; 4096];
        loop {
            // SAFETY: feedback is writable for the synchronous decoder call.
            let length = unsafe {
                deskfuse_roc_receiver_pop_feedback(
                    self.receiver,
                    feedback.as_mut_ptr(),
                    feedback.len(),
                )
            };
            if length <= 0 {
                break;
            }
            let mut framed = Vec::with_capacity(length as usize + 1);
            framed.push(3);
            framed.extend_from_slice(&feedback[..length as usize]);
            session.send(framed)?;
        }
        Ok(samples)
    }
}

impl Drop for RocReceiverPump {
    fn drop(&mut self) {
        // SAFETY: receiver was created by the matching DLL function exactly once.
        unsafe { deskfuse_roc_receiver_destroy(self.receiver) }
    }
}

pub fn pump(
    capture: &Capture,
    session: Arc<MicrophoneSession>,
    sender: &RocSessionPump,
) -> Result<(), String> {
    sender.pump_capture(capture, &session)
}
