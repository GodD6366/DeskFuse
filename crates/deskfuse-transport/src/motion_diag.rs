//! Process-global, metadata-only samples of the datagram motion path.
//!
//! These exist to answer one question from an exported diagnostics file: is
//! forwarded mouse motion being delayed or thinned out by the transport (send
//! queue backpressure, coalescing, loss, health flaps) rather than by the
//! coordinator or the native injection worker?
//!
//! `deskfuse-engine` owns the detailed diagnostics file format. Quality samples
//! also cross the input-service IPC boundary. No coordinates,
//! display identifiers, tokens, clipboard content or credentials are recorded.
use std::{collections::VecDeque, sync::Mutex, time::SystemTime, time::UNIX_EPOCH};

use deskfuse_core::{DeviceId, TransportSessionId};

/// Bounded: roughly 30 minutes of samples at the rate-limited cadence.
const CAPACITY: usize = 2048;
const QUALITY_CAPACITY: usize = 256;

/// One rate-limited sample of a single motion link.
#[derive(Clone, Debug)]
pub struct MotionDiagRecord {
    pub timestamp_ms: u128,
    /// Why this sample exists: `send`, `receive`, or `health_lost`.
    pub event: &'static str,
    /// Truncated device identity, enough to tell peers apart in one export.
    pub peer: String,
    /// Truncated transport session identity.
    pub transport_session: String,
    /// The actual bound input route, independent of the main control connection.
    pub local_addr: String,
    pub remote_addr: String,
    pub interface_name: Option<String>,
    pub interface_kind: String,
    /// Application-level datagram health, as the coordinator sees it.
    pub ready: bool,
    pub sent: u64,
    pub received: u64,
    /// Outbound samples replaced before transmission (mailbox coalescing).
    pub send_replaced: u64,
    /// Inbound samples replaced before application (inbox coalescing).
    pub receive_replaced: u64,
    /// Samples refused because the path was unusable or the mailbox was full.
    pub dropped: u64,
    /// Queued datagrams waiting for the QUIC datagram send buffer.
    pub pending_send: usize,
    pub rtt_micros: u64,
    pub sent_packets: u64,
    pub lost_packets: u64,
    pub congestion_events: u64,
}

/// Always-on, metadata-only samples used by the in-app quality chart.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct QualityRecord {
    pub timestamp_ms: u128,
    pub rtt_micros: u64,
    pub hitches: u32,
    /// 这一秒内达到慢注入阈值的注入次数。
    pub slow_injections: u32,
    /// 这一秒采集排队耗时合计，微秒。
    pub capture_queue_micros: u64,
    /// 这一秒采集样本数。
    pub capture_count: u64,
    /// 采样时数据报运动通道是否就绪。降级到可靠通道后曲线仍要连续，因此
    /// 采样本身不依赖这个值；它只用来告诉界面当前走的是哪条通道。
    pub ready: bool,
}

static RECORDS: Mutex<VecDeque<MotionDiagRecord>> = Mutex::new(VecDeque::new());
static QUALITY_RECORDS: Mutex<VecDeque<QualityRecord>> = Mutex::new(VecDeque::new());
static DROPPED_RECORDS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Append one sample, evicting the oldest so the newest is always retained.
pub(crate) fn record(record: MotionDiagRecord) {
    use std::sync::atomic::Ordering;
    let Ok(mut records) = RECORDS.try_lock() else {
        DROPPED_RECORDS.fetch_add(1, Ordering::Relaxed);
        return;
    };
    if records.len() == CAPACITY {
        records.pop_front();
        DROPPED_RECORDS.fetch_add(1, Ordering::Relaxed);
    }
    records.push_back(record);
}

/// Copy every retained sample, plus how many were evicted before export.
#[must_use]
pub fn snapshot() -> (Vec<MotionDiagRecord>, u64) {
    use std::sync::atomic::Ordering;
    let records = RECORDS
        .lock()
        .map(|records| records.iter().cloned().collect())
        .unwrap_or_default();
    (records, DROPPED_RECORDS.load(Ordering::Relaxed))
}

pub(crate) fn record_quality(record: QualityRecord) {
    let Ok(mut records) = QUALITY_RECORDS.try_lock() else {
        return;
    };
    if records.len() == QUALITY_CAPACITY {
        records.pop_front();
    }
    records.push_back(record);
}

pub fn quality_snapshot() -> Vec<QualityRecord> {
    QUALITY_RECORDS
        .lock()
        .map(|records| records.iter().cloned().collect())
        .unwrap_or_default()
}

pub(crate) fn timestamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

pub(crate) fn short_peer(peer: DeviceId) -> String {
    peer.to_hex().chars().take(12).collect()
}

pub(crate) fn short_session(id: TransportSessionId) -> String {
    id.to_string().chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(sent: u64) -> MotionDiagRecord {
        MotionDiagRecord {
            timestamp_ms: 1,
            event: "send",
            peer: "abc".into(),
            transport_session: "def".into(),
            local_addr: "10.10.10.4:42425".into(),
            remote_addr: "10.10.10.1:42425".into(),
            interface_name: Some("bridge0".into()),
            interface_kind: "Thunderbolt".into(),
            ready: true,
            sent,
            received: 0,
            send_replaced: 0,
            receive_replaced: 0,
            dropped: 0,
            pending_send: 0,
            rtt_micros: 0,
            sent_packets: 0,
            lost_packets: 0,
            congestion_events: 0,
        }
    }

    #[test]
    fn snapshot_keeps_the_newest_samples_and_counts_evictions() {
        let (before, dropped_before) = snapshot();
        for index in 0..CAPACITY + 3 {
            record(sample(index as u64));
        }
        let (records, dropped) = snapshot();
        assert_eq!(records.len(), CAPACITY.min(before.len() + CAPACITY + 3));
        assert!(records.len() <= CAPACITY);
        assert_eq!(records.last().map(|r| r.sent), Some((CAPACITY + 2) as u64));
        assert!(dropped >= dropped_before);
        // Metadata only: truncated identifiers, never the full device identity.
        assert!(records.last().is_some_and(|r| r.peer.len() <= 12));
    }

    #[test]
    fn identifiers_are_truncated() {
        let peer = DeviceId::from_bytes([7; 32]);
        assert_eq!(short_peer(peer).len(), 12);
        assert_eq!(short_session(TransportSessionId::new()).len(), 8);
    }
}
