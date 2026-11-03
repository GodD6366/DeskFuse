//! Windows native microphone capture.
//!
//! The callback owns only a bounded PCM queue. It never waits on QUIC, IPC, or
//! the UI; the service/engine layer can consume frames and encode them for the
//! authenticated microphone transport.

use cpal::{Device, FromSample, Sample, SampleFormat, Stream, StreamConfig, traits::*};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    },
    time::Duration,
};

const FRAME_SAMPLES: usize = 480;
const QUEUE_FRAMES: usize = 32;

#[derive(Clone, Debug, Default)]
pub struct InputDeviceInfo {
    pub name: String,
    pub default: bool,
}

#[derive(Debug)]
pub enum CaptureError {
    NoInputDevice,
    Device(String),
    Config(String),
    Build(String),
    Play(String),
}
impl std::fmt::Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoInputDevice => f.write_str("Windows 没有可用的麦克风输入设备"),
            Self::Device(e) => write!(f, "无法读取 Windows 麦克风设备：{e}"),
            Self::Config(e) => write!(f, "Windows 麦克风配置不可用：{e}"),
            Self::Build(e) => write!(f, "无法创建 Windows 麦克风采集流：{e}"),
            Self::Play(e) => write!(f, "无法启动 Windows 麦克风采集流：{e}"),
        }
    }
}
impl std::error::Error for CaptureError {}

pub fn input_devices() -> Result<Vec<InputDeviceInfo>, CaptureError> {
    let host = cpal::default_host();
    let default_name = host.default_input_device().and_then(|d| d.name().ok());
    host.input_devices()
        .map_err(|e| CaptureError::Device(e.to_string()))?
        .filter_map(|device| {
            let name = device.name().ok()?;
            Some(InputDeviceInfo {
                default: default_name.as_deref() == Some(name.as_str()),
                name,
            })
        })
        .collect::<Vec<_>>()
        .pipe(Ok)
}

pub struct Capture {
    stream: Stream,
    frames: Receiver<Vec<i16>>,
    level: Arc<AtomicU32>,
    stopped: Arc<AtomicBool>,
}
impl Capture {
    pub fn start(device_name: Option<&str>) -> Result<Self, CaptureError> {
        let host = cpal::default_host();
        let device = select_device(&host, device_name)?;
        let supported = device
            .default_input_config()
            .map_err(|e| CaptureError::Config(e.to_string()))?;
        let sample_format = supported.sample_format();
        let mut config: StreamConfig = supported.into();
        config.channels = 1;
        // Keep the device's native rate. The Roc encoder will resample to its
        // fixed wire rate; forcing 48 kHz here makes many USB microphones fail
        // to open on Windows because their shared mode format is 44.1 kHz.
        let (tx, frames) = mpsc::sync_channel(QUEUE_FRAMES);
        let level = Arc::new(AtomicU32::new(0.0f32.to_bits()));
        let stopped = Arc::new(AtomicBool::new(false));
        let callback_level = level.clone();
        let callback_stopped = stopped.clone();
        let error = move |error| {
            callback_stopped.store(true, Ordering::Release);
            eprintln!("DeskFuse Windows microphone stream error: {error}");
        };
        let stream = match sample_format {
            SampleFormat::I8 => build::<i8>(&device, &config, tx, callback_level, error),
            SampleFormat::I16 => build::<i16>(&device, &config, tx, callback_level, error),
            SampleFormat::I32 => build::<i32>(&device, &config, tx, callback_level, error),
            SampleFormat::F32 => build::<f32>(&device, &config, tx, callback_level, error),
            format => Err(format!("不支持的采样格式 {format:?}")),
        }
        .map_err(CaptureError::Build)?;
        stream
            .play()
            .map_err(|e| CaptureError::Play(e.to_string()))?;
        Ok(Self {
            stream,
            frames,
            level,
            stopped,
        })
    }
    pub fn try_frame(&self) -> Result<Option<Vec<i16>>, CaptureError> {
        match self.frames.try_recv() {
            Ok(frame) => Ok(Some(frame)),
            Err(TryRecvError::Empty) => Ok(None),
            Err(TryRecvError::Disconnected) => Err(CaptureError::Build("采集队列已断开".into())),
        }
    }
    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Acquire)).clamp(0.0, 1.0)
    }
    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        self.stop();
        let _ = &self.stream;
    }
}

/// PCM playback sink. When the signed DeskFuse virtual input driver is
/// installed, its exposed endpoint is selected by name and this sink feeds it
/// through the normal Windows shared-mode output path.
pub struct Playback {
    stream: Stream,
    queue: Arc<Mutex<VecDeque<f32>>>,
    stopped: Arc<AtomicBool>,
}
impl Playback {
    pub fn start(device_name: Option<&str>) -> Result<Self, CaptureError> {
        let host = cpal::default_host();
        let device = select_output_device(&host, device_name)?;
        let supported = device
            .default_output_config()
            .map_err(|e| CaptureError::Config(e.to_string()))?;
        let sample_format = supported.sample_format();
        let config: StreamConfig = supported.into();
        let queue = Arc::new(Mutex::new(VecDeque::with_capacity(
            FRAME_SAMPLES * QUEUE_FRAMES,
        )));
        let stopped = Arc::new(AtomicBool::new(false));
        let callback_queue = queue.clone();
        let callback_stopped = stopped.clone();
        let error = move |error| {
            callback_stopped.store(true, Ordering::Release);
            eprintln!("DeskFuse Windows microphone playback error: {error}");
        };
        let stream = match sample_format {
            SampleFormat::I8 => build_output::<i8>(&device, &config, callback_queue, error),
            SampleFormat::I16 => build_output::<i16>(&device, &config, callback_queue, error),
            SampleFormat::I32 => build_output::<i32>(&device, &config, callback_queue, error),
            SampleFormat::F32 => build_output::<f32>(&device, &config, callback_queue, error),
            format => Err(format!("不支持的输出采样格式 {format:?}")),
        }
        .map_err(CaptureError::Build)?;
        stream
            .play()
            .map_err(|e| CaptureError::Play(e.to_string()))?;
        Ok(Self {
            stream,
            queue,
            stopped,
        })
    }
    pub fn push(&self, samples: &[f32]) {
        let Ok(mut queue) = self.queue.lock() else {
            return;
        };
        let capacity = FRAME_SAMPLES * QUEUE_FRAMES;
        let keep = samples.len().min(capacity);
        while queue.len() + keep > capacity {
            queue.pop_front();
        }
        queue.extend(
            samples[samples.len().saturating_sub(keep)..]
                .iter()
                .copied(),
        );
    }
    pub fn is_stopped(&self) -> bool {
        self.stopped.load(Ordering::Acquire)
    }
}
impl Drop for Playback {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        let _ = &self.stream;
    }
}

fn select_device(host: &cpal::Host, wanted: Option<&str>) -> Result<Device, CaptureError> {
    if let Some(wanted) = wanted.filter(|name| !name.is_empty()) {
        let device = host
            .input_devices()
            .map_err(|e| CaptureError::Device(e.to_string()))?
            .find(|device| device.name().ok().as_deref() == Some(wanted));
        return device.ok_or_else(|| CaptureError::Device(format!("找不到输入设备 {wanted}")));
    }
    host.default_input_device()
        .ok_or(CaptureError::NoInputDevice)
}

fn select_output_device(host: &cpal::Host, wanted: Option<&str>) -> Result<Device, CaptureError> {
    if let Some(wanted) = wanted.filter(|name| !name.is_empty()) {
        let device = host
            .output_devices()
            .map_err(|e| CaptureError::Device(e.to_string()))?
            .find(|device| device.name().ok().as_deref() == Some(wanted));
        return device.ok_or_else(|| CaptureError::Device(format!("找不到输出设备 {wanted}")));
    }
    host.default_output_device()
        .ok_or(CaptureError::NoInputDevice)
}

fn build<T: Sample + FromSample<f32> + Send + 'static>(
    device: &Device,
    config: &StreamConfig,
    tx: SyncSender<Vec<i16>>,
    level: Arc<AtomicU32>,
    error: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<Stream, String> {
    let mut pending = Vec::with_capacity(FRAME_SAMPLES);
    device
        .build_input_stream(
            config,
            move |data: &[T], _| {
                let mut peak = 0.0f32;
                for sample in data {
                    let value = sample.to_sample::<f32>();
                    peak = peak.max(value.abs());
                    pending.push((value.clamp(-1.0, 1.0) * i16::MAX as f32) as i16);
                    if pending.len() == FRAME_SAMPLES {
                        let frame =
                            std::mem::replace(&mut pending, Vec::with_capacity(FRAME_SAMPLES));
                        if let Err(TrySendError::Disconnected(_)) = tx.try_send(frame) {
                            return;
                        }
                    }
                }
                level.store(peak.to_bits(), Ordering::Release);
            },
            error,
            Some(Duration::from_millis(100)),
        )
        .map_err(|e| e.to_string())
}

fn build_output<T: Sample + FromSample<f32> + Send + 'static>(
    device: &Device,
    config: &StreamConfig,
    queue: Arc<Mutex<VecDeque<f32>>>,
    error: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<Stream, String> {
    let channels = usize::from(config.channels.max(1));
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                let Ok(mut queue) = queue.lock() else {
                    for sample in data.iter_mut() {
                        *sample = T::from_sample(0.0);
                    }
                    return;
                };
                for frame in data.chunks_mut(channels) {
                    let value = queue.pop_front().unwrap_or(0.0);
                    for sample in frame {
                        *sample = T::from_sample(value);
                    }
                }
            },
            error,
            Some(Duration::from_millis(100)),
        )
        .map_err(|e| e.to_string())
}

trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}
impl<T> Pipe for T {}
