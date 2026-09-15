use crate::{DisplayId, InputEvent};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Policy used when a sequence number is not greater than the previous one.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SequencePolicy {
    /// Silently discard the stale event and record it in batch statistics.
    #[default]
    DropOutOfOrder,
    /// Return [`BatchError::OutOfOrder`] to the caller.
    RejectOutOfOrder,
}

/// Batching and compression policy.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BatchPolicy {
    /// Coalesce adjacent motion events for the same display.
    pub compress_motion: bool,
    /// Maximum number of events before a consumer should flush the batch.
    pub max_events: usize,
    /// Handling for duplicate or out-of-order sequences.
    pub sequence_policy: SequencePolicy,
}

impl Default for BatchPolicy {
    fn default() -> Self {
        Self {
            compress_motion: true,
            max_events: 64,
            sequence_policy: SequencePolicy::DropOutOfOrder,
        }
    }
}

impl BatchPolicy {
    /// Creates a policy with a maximum event count.
    #[must_use]
    pub const fn with_max_events(max_events: usize) -> Self {
        Self {
            compress_motion: true,
            max_events,
            sequence_policy: SequencePolicy::DropOutOfOrder,
        }
    }

    /// Sets the sequence policy.
    #[must_use]
    pub const fn with_sequence_policy(mut self, sequence_policy: SequencePolicy) -> Self {
        self.sequence_policy = sequence_policy;
        self
    }

    /// Enables or disables adjacent-motion compression.
    #[must_use]
    pub const fn with_motion_compression(mut self, enabled: bool) -> Self {
        self.compress_motion = enabled;
        self
    }
}

/// Why an event was discarded.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DropReason {
    /// Sequence was equal to or lower than the last accepted sequence.
    OutOfOrder,
}

/// Result of passing one event to an [`InputBatcher`].
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PushOutcome {
    /// The event was appended.
    Accepted,
    /// The event replaced a preceding motion event for the same display.
    CoalescedMotion,
    /// The event was not accepted.
    Dropped(DropReason),
}

/// A missing sequence range observed between accepted events.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SequenceGap {
    /// First missing sequence number, inclusive.
    pub first: u64,
    /// Last missing sequence number, inclusive.
    pub last: u64,
}

/// Errors when constructing a strictly ordered batch.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BatchError {
    /// Event sequence was not greater than the prior event.
    OutOfOrder {
        /// Last accepted sequence number.
        previous: u64,
        /// Received stale sequence number.
        received: u64,
    },
}

impl fmt::Display for BatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfOrder { previous, received } => write!(
                formatter,
                "input sequence {received} is not after previous sequence {previous}"
            ),
        }
    }
}

impl std::error::Error for BatchError {}

/// An ordered group of input events and observed packet gaps.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct InputBatch {
    events: Vec<InputEvent>,
    gaps: Vec<SequenceGap>,
}

impl InputBatch {
    /// Creates an empty batch.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            events: Vec::new(),
            gaps: Vec::new(),
        }
    }

    /// Creates a strictly ordered batch from events.
    pub fn from_events(events: impl IntoIterator<Item = InputEvent>) -> Result<Self, BatchError> {
        let mut batch = Self::new();
        for event in events {
            batch.push_strict(event)?;
        }
        Ok(batch)
    }

    /// Returns events in accepted wire order.
    #[must_use]
    pub fn events(&self) -> &[InputEvent] {
        &self.events
    }

    /// Consumes the batch and returns its events.
    #[must_use]
    pub fn into_events(self) -> Vec<InputEvent> {
        self.events
    }

    /// Returns observed missing sequence ranges.
    #[must_use]
    pub fn gaps(&self) -> &[SequenceGap] {
        &self.gaps
    }

    /// Number of accepted events.
    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Whether there are no accepted events.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Sequence of the first event, if any.
    #[must_use]
    pub fn first_sequence(&self) -> Option<u64> {
        self.events.first().map(InputEvent::sequence)
    }

    /// Sequence of the last event, if any.
    #[must_use]
    pub fn last_sequence(&self) -> Option<u64> {
        self.events.last().map(InputEvent::sequence)
    }

    fn push_strict(&mut self, event: InputEvent) -> Result<(), BatchError> {
        if let Some(previous) = self.last_sequence() {
            let sequence = event.sequence();
            if sequence <= previous {
                return Err(BatchError::OutOfOrder {
                    previous,
                    received: sequence,
                });
            }
            if sequence > previous.saturating_add(1) {
                self.gaps.push(SequenceGap {
                    first: previous.saturating_add(1),
                    last: sequence - 1,
                });
            }
        }
        self.events.push(event);
        Ok(())
    }

    /// Compresses adjacent motion events while preserving event order and all
    /// non-motion events.
    #[must_use]
    pub fn compress_motion(events: Vec<InputEvent>) -> Vec<InputEvent> {
        let mut output = Vec::with_capacity(events.len());
        for event in events {
            let can_replace = match (&output.last(), &event) {
                (
                    Some(InputEvent::Motion {
                        display_id: previous_display,
                        ..
                    }),
                    InputEvent::Motion { display_id, .. },
                ) => previous_display == display_id,
                _ => false,
            };
            if can_replace {
                let last = output.len() - 1;
                output[last] = event;
            } else {
                output.push(event);
            }
        }
        output
    }
}

/// Mutable event collector implementing ordering, loss accounting, and
/// adjacent-motion compression.
#[derive(Clone, Debug, PartialEq)]
pub struct InputBatcher {
    policy: BatchPolicy,
    batch: InputBatch,
    stats: BatchStats,
    /// Last accepted sequence across flush boundaries.
    last_sequence: Option<u64>,
}

impl Default for InputBatcher {
    fn default() -> Self {
        Self::new(BatchPolicy::default())
    }
}

impl InputBatcher {
    /// Creates a batcher using the supplied policy.
    #[must_use]
    pub fn new(policy: BatchPolicy) -> Self {
        Self {
            policy,
            batch: InputBatch::new(),
            stats: BatchStats::default(),
            last_sequence: None,
        }
    }

    /// Returns the configured policy.
    #[must_use]
    pub const fn policy(&self) -> BatchPolicy {
        self.policy
    }

    /// Returns the last accepted sequence across all flushed batches.
    #[must_use]
    pub const fn last_sequence(&self) -> Option<u64> {
        self.last_sequence
    }

    /// Resets ordering state, normally after a new input session is
    /// authenticated. The current pending batch is discarded.
    pub fn reset_sequence(&mut self) {
        self.batch = InputBatch::new();
        self.last_sequence = None;
    }

    /// Pushes one event, accounting for sequence gaps and stale packets.
    pub fn push(&mut self, event: InputEvent) -> Result<PushOutcome, BatchError> {
        if let Some(previous) = self.last_sequence {
            let sequence = event.sequence();
            if sequence <= previous {
                self.stats.dropped_out_of_order += 1;
                if self.policy.sequence_policy == SequencePolicy::RejectOutOfOrder {
                    return Err(BatchError::OutOfOrder {
                        previous,
                        received: sequence,
                    });
                }
                return Ok(PushOutcome::Dropped(DropReason::OutOfOrder));
            }
            if sequence > previous.saturating_add(1) {
                self.batch.gaps.push(SequenceGap {
                    first: previous.saturating_add(1),
                    last: sequence - 1,
                });
                self.stats.missing_sequences += sequence - previous - 1;
            }
        }

        self.stats.accepted += 1;
        self.last_sequence = Some(event.sequence());
        if self.policy.compress_motion {
            let can_replace = match (self.batch.events.last(), &event) {
                (
                    Some(InputEvent::Motion {
                        display_id: previous_display,
                        ..
                    }),
                    InputEvent::Motion { display_id, .. },
                ) => previous_display == display_id,
                _ => false,
            };
            if can_replace {
                let last = self.batch.events.len() - 1;
                self.batch.events[last] = event;
                self.stats.coalesced_motion += 1;
                return Ok(PushOutcome::CoalescedMotion);
            }
        }
        self.batch.events.push(event);
        Ok(PushOutcome::Accepted)
    }

    /// Returns whether the collector reached its configured event threshold.
    #[must_use]
    pub fn is_ready(&self) -> bool {
        self.policy.max_events > 0 && self.batch.len() >= self.policy.max_events
    }

    /// Returns the current batch without clearing it.
    #[must_use]
    pub fn peek(&self) -> &InputBatch {
        &self.batch
    }

    /// Takes and resets the current batch.
    pub fn flush(&mut self) -> InputBatch {
        std::mem::take(&mut self.batch)
    }

    /// Takes the current batch only when it reached the configured threshold.
    pub fn flush_if_ready(&mut self) -> Option<InputBatch> {
        self.is_ready().then(|| self.flush())
    }

    /// Returns and resets accumulated statistics.
    #[must_use]
    pub fn take_stats(&mut self) -> BatchStats {
        std::mem::take(&mut self.stats)
    }

    /// Returns accumulated statistics without resetting them.
    #[must_use]
    pub const fn stats(&self) -> BatchStats {
        self.stats
    }
}

/// Batch accounting useful for transport metrics and diagnostics.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct BatchStats {
    /// Number of accepted packets.
    pub accepted: u64,
    /// Number of stale/duplicate packets dropped.
    pub dropped_out_of_order: u64,
    /// Number of missing sequence numbers inferred from gaps.
    pub missing_sequences: u64,
    /// Number of adjacent motion packets coalesced.
    pub coalesced_motion: u64,
}

/// Returns whether two motion events target the same display. Kept public for
/// adapters which maintain a transport-specific batcher.
#[must_use]
pub fn same_motion_display(first: &InputEvent, second: &InputEvent) -> bool {
    match (first, second) {
        (
            InputEvent::Motion {
                display_id: first_display,
                ..
            },
            InputEvent::Motion {
                display_id: second_display,
                ..
            },
        ) => first_display == second_display,
        _ => false,
    }
}

// Keep this import in the public module's generated docs, and make accidental
// API changes to the display identity visible during review.
#[allow(dead_code)]
fn _display_id_type(_: &DisplayId) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{InputEvent, Point};

    #[test]
    fn compresses_adjacent_motion_but_not_across_button() {
        let events = vec![
            InputEvent::motion("main", Point::new(1.0, 1.0), 1, 1),
            InputEvent::motion("main", Point::new(2.0, 2.0), 2, 2),
            InputEvent::button(crate::ButtonCode::LEFT, true, 3, 3),
            InputEvent::motion("main", Point::new(3.0, 3.0), 4, 4),
        ];
        let compressed = InputBatch::compress_motion(events);
        assert_eq!(compressed.len(), 3);
        assert_eq!(compressed[0].sequence(), 2);
        assert_eq!(compressed[2].sequence(), 4);
    }

    #[test]
    fn records_gaps_and_drops_old_packets() {
        let mut batcher = InputBatcher::default();
        assert_eq!(
            batcher.push(InputEvent::motion("main", Point::default(), 1, 1)),
            Ok(PushOutcome::Accepted)
        );
        assert_eq!(
            batcher.push(InputEvent::motion("main", Point::default(), 3, 3)),
            Ok(PushOutcome::CoalescedMotion)
        );
        assert_eq!(
            batcher.push(InputEvent::motion("main", Point::default(), 2, 2)),
            Ok(PushOutcome::Dropped(DropReason::OutOfOrder))
        );
        assert_eq!(batcher.peek().gaps(), &[SequenceGap { first: 2, last: 2 }]);
        assert_eq!(batcher.stats().dropped_out_of_order, 1);
    }

    #[test]
    fn sequence_watermark_survives_flush() {
        let mut batcher = InputBatcher::default();
        batcher
            .push(InputEvent::button(crate::ButtonCode::LEFT, true, 1, 1))
            .expect("first event");
        let _ = batcher.flush();
        assert_eq!(batcher.last_sequence(), Some(1));
        assert_eq!(
            batcher.push(InputEvent::button(crate::ButtonCode::LEFT, false, 2, 1)),
            Ok(PushOutcome::Dropped(DropReason::OutOfOrder))
        );
    }
}
