use super::{Deadline, SYNTHETIC_SHARING_EVENT_MARKER, permission_result};
use crate::{PlatformError, PlatformResult};
#[cfg(not(test))]
use core_graphics::event::CGEventTapLocation;
use core_graphics::{
    event::{CGEvent, CGEventFlags, CGEventType, CGMouseButton, EventField, ScrollEventUnit},
    event_source::{CGEventSource, CGEventSourceStateID},
    geometry::CGPoint,
};
use foreign_types::ForeignType;
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{
        Arc, Condvar, Mutex,
        mpsc::{self, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tokio::sync::Notify;
use unidesk_core::{DisplayDescriptor, InputSessionId, SharedInputEvent};

// The pinned core-graphics crate does not expose these Quartz setters.
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventSetLocation(event: core_graphics::sys::CGEventRef, location: CGPoint);
    fn CGEventSourceSetUserData(source: core_graphics::sys::CGEventSourceRef, user_data: i64);
}

type Reply = SyncSender<PlatformResult<()>>;
const MAILBOX_CAPACITY: usize = 256;

const WAIT_SAMPLE_CAPACITY: usize = 512;

/// Metadata-only counters for this injector's lifetime. Waiting is measured
/// locally from accepted enqueue to immediately before the worker's native
/// injection call, NOT until the pointer becomes visible on screen.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct InputInjectionDiagnostics {
    pub enqueued_batches: u64,
    pub enqueued_events: u64,
    pub coalesced_events: u64,
    pub executed_events: u64,
    /// Pending queue only; excludes a batch already taken by the worker.
    pub queued_batches: usize,
    pub queued_events: usize,
    pub queue_wait_samples: u64,
    pub queue_wait_max_us: u64,
    /// Nearest-rank percentiles of the most recent at most 512 attempts.
    pub queue_wait_recent_samples: usize,
    pub queue_wait_p95_us: u64,
    pub queue_wait_p99_us: u64,
}

struct InjectionMetrics {
    counters: InputInjectionDiagnostics,
    waits_us: [u64; WAIT_SAMPLE_CAPACITY],
    next_wait: usize,
}
impl Default for InjectionMetrics {
    fn default() -> Self {
        Self {
            counters: InputInjectionDiagnostics::default(),
            waits_us: [0; WAIT_SAMPLE_CAPACITY],
            next_wait: 0,
        }
    }
}
impl InjectionMetrics {
    fn accepted(&mut self, events: usize, coalesced: usize) {
        self.counters.enqueued_batches = self.counters.enqueued_batches.saturating_add(1);
        self.counters.enqueued_events = self.counters.enqueued_events.saturating_add(events as u64);
        self.counters.coalesced_events = self
            .counters
            .coalesced_events
            .saturating_add(coalesced as u64);
    }
    fn observe_wait(&mut self, wait: Duration) {
        let micros = u64::try_from(wait.as_micros()).unwrap_or(u64::MAX);
        self.waits_us[self.next_wait] = micros;
        self.next_wait = (self.next_wait + 1) % WAIT_SAMPLE_CAPACITY;
        self.counters.queue_wait_samples = self.counters.queue_wait_samples.saturating_add(1);
        self.counters.queue_wait_recent_samples =
            (self.counters.queue_wait_recent_samples + 1).min(WAIT_SAMPLE_CAPACITY);
        self.counters.queue_wait_max_us = self.counters.queue_wait_max_us.max(micros);
    }
    fn snapshot(&self) -> InputInjectionDiagnostics {
        let mut result = self.counters.clone();
        let count = result.queue_wait_recent_samples;
        if count != 0 {
            let mut samples = self.waits_us[..count].to_vec();
            samples.sort_unstable();
            result.queue_wait_p95_us = samples[(count * 95).div_ceil(100) - 1];
            result.queue_wait_p99_us = samples[(count * 99).div_ceil(100) - 1];
        }
        result
    }
}

struct PendingInput {
    enqueued_at: Instant,
    generation: u64,
    session: InputSessionId,
    display: DisplayDescriptor,
    event: SharedInputEvent,
    remaining_events: Vec<SharedInputEvent>,
    // None keeps legacy callers separate from explicitly versioned batches.
    state_epoch: Option<u64>,
    reply: Option<Reply>,
}

impl PendingInput {
    fn events(&self) -> impl Iterator<Item = &SharedInputEvent> {
        std::iter::once(&self.event).chain(&self.remaining_events)
    }
    fn event_count(&self) -> usize {
        1 + self.remaining_events.len()
    }
    fn pure_motion(&self) -> bool {
        self.events()
            .all(|event| matches!(event, SharedInputEvent::Motion { .. }))
    }
    fn validate_batch(&self) -> PlatformResult<()> {
        if !self.display.frame_x.is_finite() || !self.display.frame_y.is_finite() {
            return Err(PlatformError::InputInjectionFailed);
        }
        for event in self.events() {
            event
                .validate()
                .map_err(|_| PlatformError::InputInjectionFailed)?;
            if let SharedInputEvent::Motion { x, y } = event
                && !self.display.contains(*x, *y)
            {
                return Err(PlatformError::InputInjectionFailed);
            }
        }
        Ok(())
    }
}

#[derive(Default)]
struct MailboxState {
    metrics: InjectionMetrics,
    inputs: VecDeque<PendingInput>,
    generation: u64,
    session: Option<InputSessionId>,
    release_pending: bool,
    release_replies: Vec<Reply>,
    error: Option<PlatformError>,
    stopped: bool,
}
impl MailboxState {
    fn invalidate(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.session = None;
        self.release_pending = true;
        for input in self.inputs.drain(..) {
            if let Some(reply) = input.reply {
                let _ = reply.try_send(Err(PlatformError::InputInjectionDisabled));
            }
        }
    }
    fn fail(&mut self, error: PlatformError) {
        self.invalidate();
        // Keep the first cause until the coordinator handles it.
        self.error.get_or_insert(error);
    }
    fn expire(&mut self, deadline: &Deadline) -> bool {
        if self.session.is_some() && deadline.expired() {
            self.fail(PlatformError::InputInjectionDisabled);
            deadline.clear();
            true
        } else {
            false
        }
    }
    fn accepts(&self, input: &PendingInput) -> bool {
        !self.stopped
            && self.error.is_none()
            && self.generation == input.generation
            && self.session == Some(input.session)
    }
    fn push(&mut self, mut input: PendingInput) -> PlatformResult<()> {
        if !self.accepts(&input) {
            return Err(PlatformError::InputInjectionDisabled);
        }
        let submitted_events = input.event_count();
        // Validate the complete versioned batch before replacing or appending
        // anything. Failure retires all work, never admitting a valid prefix.
        if input.state_epoch.is_some() {
            if let Err(error) = input.validate_batch() {
                self.fail(error.clone());
                return Err(error);
            }
            if input.event_count() > MAILBOX_CAPACITY {
                self.fail(PlatformError::InputInjectionFailed);
                return Err(PlatformError::InputInjectionFailed);
            }
            if input.pure_motion()
                && let Some(latest) = input.remaining_events.pop()
            {
                input.event = latest;
                input.remaining_events.clear();
            }
        }
        // Absolute positions are replaceable, but never move them across a
        // button, key, scroll, display, session, or synchronous-call boundary.
        if input.reply.is_none()
            && input.pure_motion()
            && let Some(last) = self.inputs.back_mut()
            && last.reply.is_none()
            && last.generation == input.generation
            && last.session == input.session
            && last.display == input.display
            && last.state_epoch == input.state_epoch
            && last.pure_motion()
        {
            let coalesced = submitted_events - input.event_count() + last.event_count();
            std::mem::swap(&mut last.event, &mut input.event);
            last.enqueued_at = input.enqueued_at;
            self.metrics.accepted(submitted_events, coalesced);
            return Ok(());
        }
        if self
            .inputs
            .iter()
            .map(PendingInput::event_count)
            .sum::<usize>()
            + input.event_count()
            > MAILBOX_CAPACITY
        {
            self.fail(PlatformError::InputInjectionFailed);
            return Err(PlatformError::InputInjectionFailed);
        }
        self.metrics
            .accepted(submitted_events, submitted_events - input.event_count());
        self.inputs.push_back(input);
        Ok(())
    }
}
#[derive(Default)]
struct Mailbox {
    state: Mutex<MailboxState>,
    wake: Condvar,
    // Control boundaries serialize with native posting, not producer enqueue.
    // No native permission query or per-event acknowledgement blocks enqueue.
    execution: Mutex<()>,
    errors: Arc<Notify>,
}

enum Work {
    Batch(PendingInput),
    Release(Vec<Reply>),
}

/// Quartz and its pressed-state ledger remain confined to this worker.
/// Producers wake it immediately and only retain the latest adjacent motion.
pub struct MacOSSharingInjector {
    mailbox: Arc<Mailbox>,
    deadline: Arc<Deadline>,
    thread: Option<JoinHandle<()>>,
}
impl MacOSSharingInjector {
    pub fn new() -> PlatformResult<Self> {
        permission_result()?;
        let mailbox = Arc::new(Mailbox::default());
        let deadline = Arc::new(Deadline::new());
        let worker_mailbox = mailbox.clone();
        let worker_deadline = deadline.clone();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("unidesk-sharing-injector".into())
            .spawn(move || {
                super::prioritize_current_thread();
                let mut native = match NativeInjector::new() {
                    Ok(native) => native,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(()));
                run_worker(&worker_mailbox, &worker_deadline, &mut native);
                let _ = native.release_all();
            })
            .map_err(|_| PlatformError::InputInjectionFailed)?;
        if matches!(ready_rx.recv_timeout(Duration::from_secs(2)), Ok(Ok(()))) {
            Ok(Self {
                mailbox,
                deadline,
                thread: Some(thread),
            })
        } else {
            if let Ok(mut state) = mailbox.state.lock() {
                state.stopped = true;
            }
            mailbox.wake.notify_one();
            let _ = thread.join();
            Err(PlatformError::InputInjectionFailed)
        }
    }
    pub fn arm_session(&self, session: InputSessionId, duration: Duration) -> bool {
        // Permission is checked on entry, never in the per-event worker. The
        // coordinator's independent observer gates hot input with fresh actual
        // permission/session observations and reclaims on revocation.
        if let Err(error) = permission_result() {
            self.deadline.clear();
            if let Ok(mut state) = self.mailbox.state.lock() {
                state.fail(error);
            }
            self.mailbox.errors.notify_one();
            self.mailbox.wake.notify_one();
            return false;
        }
        let Ok(_execution) = self.mailbox.execution.lock() else {
            return false;
        };
        let Ok(mut state) = self.mailbox.state.lock() else {
            return false;
        };
        if state.stopped || state.error.is_some() || !self.deadline.arm(session, duration) {
            return false;
        }
        // Even re-entry into the same leased session starts a new native epoch.
        state.invalidate();
        state.session = Some(session);
        self.mailbox.wake.notify_one();
        true
    }
    pub fn renew_deadline(&self, duration: Duration) -> bool {
        self.deadline.renew(duration)
    }
    pub fn lease_expired(&self) -> bool {
        self.deadline.expired()
    }

    /// Non-blocking with respect to native injection and permission sampling.
    /// Queue saturation fails closed rather than dropping a key/button edge.
    pub fn enqueue(
        &self,
        session_id: InputSessionId,
        display: &DisplayDescriptor,
        event: &SharedInputEvent,
    ) -> PlatformResult<()> {
        self.queue(Some(session_id), display, event, None)
    }
    /// Atomically queue a versioned injection batch. Anchors and their state
    /// edges execute under one native execution lock; only pure-motion batches
    /// in the same epoch can replace one another. Success acknowledges enqueue,
    /// not Quartz completion. Native failures still release all held input.
    pub fn enqueue_batch(
        &self,
        session_id: InputSessionId,
        display: &DisplayDescriptor,
        state_epoch: u64,
        events: &[SharedInputEvent],
    ) -> PlatformResult<()> {
        self.queue_input(Some(session_id), display, events, Some(state_epoch), None)
    }
    /// Aggregate queue/worker telemetry only; never includes input values.
    /// Percentiles use a bounded recent sample window, while max/counts span
    /// this injector's lifetime, including session changes and releases.
    pub fn diagnostics(&self) -> InputInjectionDiagnostics {
        let state = self
            .mailbox
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut result = state.metrics.snapshot();
        result.queued_batches = state.inputs.len();
        result.queued_events = state.inputs.iter().map(PendingInput::event_count).sum();
        result
    }
    pub fn error_notification(&self) -> Arc<Notify> {
        self.mailbox.errors.clone()
    }
    pub fn take_error(&self) -> Option<PlatformError> {
        self.mailbox.state.lock().ok()?.error.take()
    }
    /// Compatibility API for callers that explicitly require native completion.
    pub fn inject(
        &self,
        display: &DisplayDescriptor,
        event: &SharedInputEvent,
    ) -> PlatformResult<()> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.queue(None, display, event, Some(tx))?;
        self.wait_reply(rx)
    }
    fn queue(
        &self,
        session: Option<InputSessionId>,
        display: &DisplayDescriptor,
        event: &SharedInputEvent,
        reply: Option<Reply>,
    ) -> PlatformResult<()> {
        self.queue_input(session, display, std::slice::from_ref(event), None, reply)
    }
    fn queue_input(
        &self,
        session: Option<InputSessionId>,
        display: &DisplayDescriptor,
        events: &[SharedInputEvent],
        state_epoch: Option<u64>,
        reply: Option<Reply>,
    ) -> PlatformResult<()> {
        let mut state = self
            .mailbox
            .state
            .lock()
            .map_err(|_| PlatformError::InputInjectionFailed)?;
        let session = session
            .or(state.session)
            .ok_or(PlatformError::InputInjectionDisabled)?;
        if self.deadline.expired() {
            return Err(PlatformError::InputInjectionDisabled);
        }
        if events.is_empty() || events.len() > MAILBOX_CAPACITY {
            state.fail(PlatformError::InputInjectionFailed);
            self.deadline.clear();
            self.mailbox.errors.notify_one();
            self.mailbox.wake.notify_one();
            return Err(PlatformError::InputInjectionFailed);
        }
        let event = &events[0];
        let remaining_events = &events[1..];
        let generation = state.generation;
        let result = state.push(PendingInput {
            enqueued_at: Instant::now(),
            generation,
            session,
            display: display.clone(),
            event: event.clone(),
            remaining_events: remaining_events.to_vec(),
            state_epoch,
            reply,
        });
        if state.error.is_some() {
            self.deadline.clear();
            self.mailbox.errors.notify_one();
        }
        self.mailbox.wake.notify_one();
        result
    }
    pub fn release_all(&self) -> PlatformResult<()> {
        let (tx, rx) = mpsc::sync_channel(1);
        {
            let _execution = self
                .mailbox
                .execution
                .lock()
                .map_err(|_| PlatformError::InputInjectionFailed)?;
            let mut state = self
                .mailbox
                .state
                .lock()
                .map_err(|_| PlatformError::InputInjectionFailed)?;
            self.deadline.pause();
            state.invalidate();
            if state.release_replies.len() >= MAILBOX_CAPACITY {
                return Err(PlatformError::InputInjectionFailed);
            }
            state.release_replies.push(tx);
        }
        self.mailbox.wake.notify_one();
        self.wait_reply(rx)
    }
    /// Queue a full release without waiting for the worker.
    ///
    /// `invalidate` already sets `release_pending`, so the worker performs the
    /// native release on its next turn; waiting here only blocked the
    /// coordinator's single-threaded async `select!` behind the worker's
    /// execution lock, which stalled unrelated capture and injection branches.
    /// A native failure still reaches the coordinator through
    /// `error_notification` and `take_error`.
    pub fn release_all_nowait(&self) -> PlatformResult<()> {
        {
            let mut state = self
                .mailbox
                .state
                .lock()
                .map_err(|_| PlatformError::InputInjectionFailed)?;
            self.deadline.pause();
            state.invalidate();
        }
        self.mailbox.wake.notify_one();
        Ok(())
    }
    fn wait_reply(&self, rx: mpsc::Receiver<PlatformResult<()>>) -> PlatformResult<()> {
        rx.recv_timeout(Duration::from_millis(250))
            .unwrap_or_else(|_| {
                self.deadline.clear();
                if let Ok(mut state) = self.mailbox.state.lock() {
                    state.fail(PlatformError::InputInjectionFailed);
                }
                self.mailbox.errors.notify_one();
                self.mailbox.wake.notify_one();
                Err(PlatformError::InputInjectionFailed)
            })
    }
}
impl Drop for MacOSSharingInjector {
    fn drop(&mut self) {
        self.deadline.clear();
        if let Ok(mut state) = self.mailbox.state.lock() {
            state.invalidate();
            state.stopped = true;
        }
        self.mailbox.wake.notify_one();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run_worker(mailbox: &Mailbox, deadline: &Deadline, native: &mut NativeInjector) {
    let mut last_generation = None;
    let mut last_slow_batch: Option<Instant> = None;
    loop {
        let work = {
            let Ok(mut state) = mailbox.state.lock() else {
                break;
            };
            if state.stopped {
                break;
            }
            if state.expire(deadline) {
                mailbox.errors.notify_one();
            }
            if state.release_pending {
                state.release_pending = false;
                Work::Release(std::mem::take(&mut state.release_replies))
            } else if let Some(input) = state.inputs.pop_front() {
                Work::Batch(input)
            } else {
                // Timeout only services the independent native lease watchdog;
                // input arrival always wakes the worker immediately.
                if mailbox
                    .wake
                    .wait_timeout(state, Duration::from_millis(10))
                    .is_err()
                {
                    break;
                }
                continue;
            }
        };
        let Ok(_execution) = mailbox.execution.lock() else {
            break;
        };
        match work {
            Work::Release(replies) => {
                let result = native.release_all();
                for reply in replies {
                    let _ = reply.try_send(result.clone());
                }
                if let Err(error) = result {
                    if let Ok(mut state) = mailbox.state.lock() {
                        state.fail(error);
                        state.release_pending = false;
                    }
                    deadline.clear();
                    mailbox.errors.notify_one();
                }
            }
            Work::Batch(input) => {
                let current = mailbox
                    .state
                    .lock()
                    .is_ok_and(|state| state.accepts(&input));
                if !current {
                    if let Some(reply) = input.reply {
                        let _ = reply.try_send(Err(PlatformError::InputInjectionDisabled));
                    }
                    continue;
                }
                let first_batch = last_generation != Some(input.generation);
                last_generation = Some(input.generation);
                let debug_enabled = super::timing::enabled();
                let injection_started = debug_enabled.then(Instant::now);
                let result = inject_batch(mailbox, deadline, native, &input, debug_enabled);
                if let Some(injection_started) = injection_started {
                    let injection_time = injection_started.elapsed();
                    let queue_wait = injection_started.saturating_duration_since(input.enqueued_at);
                    let slow = queue_wait >= Duration::from_millis(16)
                        || injection_time >= Duration::from_millis(8);
                    if first_batch
                        || (slow
                            && last_slow_batch
                                .is_none_or(|last| last.elapsed() >= Duration::from_secs(1)))
                    {
                        if slow {
                            last_slow_batch = Some(Instant::now());
                        }
                        super::timing::record!(
                            if first_batch {
                                "inject_first_batch"
                            } else {
                                "inject_slow_batch"
                            },
                            injection_time,
                            format!(
                                "session={:?} epoch={:?} generation={} queue_wait_us={} events={} ok={}",
                                input.session,
                                input.state_epoch,
                                input.generation,
                                queue_wait.as_micros(),
                                input.event_count(),
                                result.is_ok()
                            ),
                        );
                    }
                }
                if let Err(error) = &result {
                    deadline.clear();
                    if let Ok(mut state) = mailbox.state.lock() {
                        state.fail(error.clone());
                    }
                    let _ = native.release_all();
                    mailbox.errors.notify_one();
                }
                if let Some(reply) = input.reply {
                    let _ = reply.try_send(result);
                }
            }
        }
    }
}
// The worker owns mailbox.execution for the entire batch. Enqueue cannot
// interleave native events, while expiry/fault invalidation is checked between
// events so a large batch cannot postpone fail-safe release.
fn inject_batch(
    mailbox: &Mailbox,
    deadline: &Deadline,
    native: &mut NativeInjector,
    input: &PendingInput,
    debug_enabled: bool,
) -> PlatformResult<()> {
    let batch_start = debug_enabled.then(Instant::now);
    let mut executed = 0_u64;
    let mut total_lock_time = Duration::ZERO;
    let mut total_inject_time = Duration::ZERO;
    let result = (|| {
        for event in input.events() {
            if deadline.expired() {
                return Err(PlatformError::InputInjectionDisabled);
            }
            {
                let lock_start = debug_enabled.then(Instant::now);
                let mut state = mailbox
                    .state
                    .lock()
                    .map_err(|_| PlatformError::InputInjectionDisabled)?;
                if !state.accepts(input) {
                    return Err(PlatformError::InputInjectionDisabled);
                }
                if debug_enabled {
                    state.metrics.observe_wait(input.enqueued_at.elapsed());
                    if let Some(lock_start) = lock_start {
                        total_lock_time += lock_start.elapsed();
                    }
                }
            }
            let inject_start = debug_enabled.then(Instant::now);
            native.inject(&input.display, event)?;
            if let Some(inject_start) = inject_start {
                total_inject_time += inject_start.elapsed();
            }
            executed += 1;
        }
        Ok(())
    })();
    // One additional bookkeeping lock per batch, outside Quartz posting. Count
    // only completed injections, including a successful prefix before failure.
    if let Ok(mut state) = mailbox.state.lock() {
        state.metrics.counters.executed_events = state
            .metrics
            .counters
            .executed_events
            .saturating_add(executed);
    }
    if let Some(batch_start) = batch_start {
        let batch_time = batch_start.elapsed();
        if batch_time >= Duration::from_millis(8)
            || total_lock_time >= Duration::from_millis(2)
            || total_inject_time >= Duration::from_millis(4)
        {
            super::timing::record!(
                "inject_batch_slow",
                batch_time,
                format!(
                    "session={:?} events={} executed={} lock={}us inject={}us",
                    input.session,
                    input.event_count(),
                    executed,
                    total_lock_time.as_micros(),
                    total_inject_time.as_micros()
                ),
            );
        }
    }
    result
}

struct NativeInjector {
    source: CGEventSource,
    position: CGPoint,
    positioned: bool,
    keys: BTreeSet<u16>,
    buttons: BTreeSet<u8>,
    modifier_keys: BTreeSet<u16>,
    modifiers: u64,
    scroll_residual: (f64, f64),
    clicks: BTreeMap<u8, (Instant, CGPoint, i64)>,
    #[cfg(test)]
    posted: std::cell::RefCell<Vec<CGEvent>>,
}
impl NativeInjector {
    fn new() -> PlatformResult<Self> {
        let source = CGEventSource::new(CGEventSourceStateID::Private)
            .map_err(|()| PlatformError::InputInjectionFailed)?;
        // Tag the source, not individual events: a per-event user-data override
        // survives CGEvent copies and prevents scroll processors such as Mos
        // from replacing it with their own recursion marker.
        // SAFETY: source is a live, exclusively initialized Quartz source on
        // this worker; user_data is an opaque integer, not a pointer.
        unsafe {
            CGEventSourceSetUserData(source.as_ptr(), SYNTHETIC_SHARING_EVENT_MARKER);
        }
        Ok(Self {
            source,
            position: CGPoint::new(0.0, 0.0),
            positioned: false,
            keys: BTreeSet::new(),
            buttons: BTreeSet::new(),
            modifier_keys: BTreeSet::new(),
            modifiers: 0,
            scroll_residual: (0.0, 0.0),
            clicks: BTreeMap::new(),
            #[cfg(test)]
            posted: std::cell::RefCell::new(Vec::new()),
        })
    }
    fn post(&self, event: CGEvent) {
        event.set_flags(CGEventFlags::from_bits_retain(self.modifiers));
        // Inject at the HID level, like real pointing hardware and like Deskflow.
        // `input_native.rs` already posts its synthetic MouseMoved at HID; session
        // level posting sits downstream of the window server's HID handling.
        #[cfg(not(test))]
        event.post(CGEventTapLocation::HID);
        #[cfg(test)]
        self.posted.borrow_mut().push(event);
    }
    fn inject(
        &mut self,
        display: &DisplayDescriptor,
        event: &SharedInputEvent,
    ) -> PlatformResult<()> {
        let inject_start = super::timing::enabled().then(Instant::now);
        event
            .validate()
            .map_err(|_| PlatformError::InputInjectionFailed)?;
        match event {
            SharedInputEvent::Motion { x, y } => {
                if !display.contains(*x, *y)
                    || !display.frame_x.is_finite()
                    || !display.frame_y.is_finite()
                {
                    return Err(PlatformError::InputInjectionFailed);
                }
                let point = CGPoint::new(display.frame_x + x, display.frame_y + y);
                let button = self.buttons.iter().next().copied();
                let kind = match button {
                    Some(0) => CGEventType::LeftMouseDragged,
                    Some(1) => CGEventType::RightMouseDragged,
                    Some(_) => CGEventType::OtherMouseDragged,
                    None => CGEventType::MouseMoved,
                };
                let event = CGEvent::new_mouse_event(
                    self.source.clone(),
                    kind,
                    point,
                    native_button(button.unwrap_or(0)),
                )
                .map_err(|()| PlatformError::InputInjectionFailed)?;
                if let Some(button) = button {
                    event.set_integer_value_field(
                        EventField::MOUSE_EVENT_BUTTON_NUMBER,
                        i64::from(button),
                    );
                }
                event.set_double_value_field(
                    EventField::MOUSE_EVENT_DELTA_X,
                    if self.positioned {
                        point.x - self.position.x
                    } else {
                        0.0
                    },
                );
                event.set_double_value_field(
                    EventField::MOUSE_EVENT_DELTA_Y,
                    if self.positioned {
                        point.y - self.position.y
                    } else {
                        0.0
                    },
                );
                self.position = point;
                self.positioned = true;
                self.post(event);
            }
            SharedInputEvent::Button { button, pressed } => {
                if !self.positioned {
                    return Err(PlatformError::InputInjectionFailed);
                }

                let count = if *pressed {
                    let now = Instant::now();
                    let count = self
                        .clicks
                        .get(button)
                        .filter(|(last, point, _)| {
                            now.duration_since(*last) <= Duration::from_millis(500)
                                && (point.x - self.position.x).hypot(point.y - self.position.y)
                                    <= 4.0
                        })
                        .map_or(1, |(_, _, count)| count + 1);
                    self.clicks.insert(*button, (now, self.position, count));
                    count
                } else {
                    self.clicks.get(button).map_or(1, |(_, _, count)| *count)
                };
                self.post_button(*button, *pressed, count)?;
                if *pressed {
                    self.buttons.insert(*button);
                } else {
                    self.buttons.remove(button);
                }
            }
            SharedInputEvent::Key {
                code,
                pressed,
                modifiers,
                repeat,
            } => {
                self.set_modifiers(*modifiers)?;

                let event = CGEvent::new_keyboard_event(self.source.clone(), *code, *pressed)
                    .map_err(|()| PlatformError::InputInjectionFailed)?;
                event.set_integer_value_field(
                    EventField::KEYBOARD_EVENT_AUTOREPEAT,
                    i64::from(*repeat),
                );
                self.post(event);
                if *pressed {
                    self.keys.insert(*code);
                } else {
                    self.keys.remove(code);
                }
            }
            SharedInputEvent::Modifiers { modifiers } => self.set_modifiers(*modifiers)?,
            SharedInputEvent::Scroll { dx, dy } => {
                // Point-delta fields are integers, while fixed-point fields
                // preserve 16.16 fractions. Carry fractional points for clients
                // which only consume the integer fields instead of losing them.
                let total_x = self.scroll_residual.0 + dx;
                let total_y = self.scroll_residual.1 + dy;
                let point_x = total_x.round() as i32;
                let point_y = total_y.round() as i32;
                self.scroll_residual = (total_x - f64::from(point_x), total_y - f64::from(point_y));
                let event = CGEvent::new_scroll_event(
                    self.source.clone(),
                    ScrollEventUnit::PIXEL,
                    2,
                    point_y,
                    point_x,
                    0,
                )
                .map_err(|()| PlatformError::InputInjectionFailed)?;
                if self.positioned {
                    // SAFETY: event is a live owned Quartz event. Position was
                    // validated by the preceding motion/entry on this worker.
                    // Scroll targets the controlled pointer, not the receiver's
                    // physical cursor position sampled when the event is made.
                    unsafe {
                        CGEventSetLocation(event.as_ptr(), self.position);
                    }
                }
                event.set_integer_value_field(EventField::SCROLL_WHEEL_EVENT_IS_CONTINUOUS, 1);
                event.set_integer_value_field(
                    EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_1,
                    i64::from(point_y),
                );
                event.set_integer_value_field(
                    EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_2,
                    i64::from(point_x),
                );
                event.set_double_value_field(
                    EventField::SCROLL_WHEEL_EVENT_FIXED_POINT_DELTA_AXIS_1,
                    *dy,
                );
                event.set_double_value_field(
                    EventField::SCROLL_WHEEL_EVENT_FIXED_POINT_DELTA_AXIS_2,
                    *dx,
                );
                self.post(event);
            }
        }
        if let Some(inject_start) = inject_start {
            let inject_time = inject_start.elapsed();
            if inject_time >= Duration::from_micros(500) {
                super::timing::record!(
                    "inject_single_slow",
                    inject_time,
                    format!("event={:?}", event),
                );
            }
        }
        Ok(())
    }
    fn post_button(&self, button: u8, pressed: bool, count: i64) -> PlatformResult<()> {
        let event = CGEvent::new_mouse_event(
            self.source.clone(),
            button_type(button, pressed),
            self.position,
            native_button(button),
        )
        .map_err(|()| PlatformError::InputInjectionFailed)?;
        event.set_integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER, i64::from(button));
        event.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, count);
        self.post(event);
        Ok(())
    }
    fn set_modifiers(&mut self, flags: u64) -> PlatformResult<()> {
        let target = modifier_keycodes(flags);
        let changes = self
            .modifier_keys
            .symmetric_difference(&target)
            .copied()
            .collect::<Vec<_>>();
        self.modifiers = flags;
        for code in changes {
            let down = target.contains(&code);
            let event = CGEvent::new_keyboard_event(self.source.clone(), code, down)
                .map_err(|()| PlatformError::InputInjectionFailed)?;
            event.set_type(CGEventType::FlagsChanged);
            self.post(event);
            if down {
                self.modifier_keys.insert(code);
            } else {
                self.modifier_keys.remove(&code);
            }
        }
        Ok(())
    }
    fn release_all(&mut self) -> PlatformResult<()> {
        let mut error = None;
        for code in self.keys.iter().copied().collect::<Vec<_>>() {
            match CGEvent::new_keyboard_event(self.source.clone(), code, false) {
                Ok(event) => {
                    self.post(event);
                    self.keys.remove(&code);
                }
                Err(()) => error = Some(PlatformError::InputInjectionFailed),
            }
        }
        for button in self.buttons.iter().copied().collect::<Vec<_>>() {
            if let Err(e) = self.post_button(button, false, 1) {
                error = Some(e);
            } else {
                self.buttons.remove(&button);
            }
        }
        if let Err(e) = self.set_modifiers(0) {
            error = Some(e);
        }
        self.clicks.clear();
        self.scroll_residual = (0.0, 0.0);
        self.positioned = false;
        error.map_or(Ok(()), Err)
    }
}
impl Drop for NativeInjector {
    fn drop(&mut self) {
        let _ = self.release_all();
    }
}
fn native_button(button: u8) -> CGMouseButton {
    match button {
        0 => CGMouseButton::Left,
        1 => CGMouseButton::Right,
        _ => CGMouseButton::Center,
    }
}
fn button_type(button: u8, pressed: bool) -> CGEventType {
    match (button, pressed) {
        (0, true) => CGEventType::LeftMouseDown,
        (0, false) => CGEventType::LeftMouseUp,
        (1, true) => CGEventType::RightMouseDown,
        (1, false) => CGEventType::RightMouseUp,
        (_, true) => CGEventType::OtherMouseDown,
        (_, false) => CGEventType::OtherMouseUp,
    }
}
/// Preserve left/right hardware modifier bits when Quartz supplies them. A
/// platform-independent aggregate flag falls back to the left-hand modifier.
pub(super) fn modifier_keycodes(flags: u64) -> BTreeSet<u16> {
    let mut keys = BTreeSet::new();
    for (aggregate, left_bit, right_bit, left_code, right_code) in [
        (1 << 17, 0x2, 0x4, 56, 60),
        (1 << 18, 0x1, 0x2000, 59, 62),
        (1 << 19, 0x20, 0x40, 58, 61),
        (1 << 20, 0x8, 0x10, 55, 54),
    ] {
        if flags & aggregate != 0 {
            if flags & left_bit != 0 || flags & (left_bit | right_bit) == 0 {
                keys.insert(left_code);
            }
            if flags & right_bit != 0 {
                keys.insert(right_code);
            }
        }
    }
    if flags & (1 << 16) != 0 {
        keys.insert(57);
    }
    if flags & (1 << 23) != 0 {
        keys.insert(63);
    }
    keys
}

impl unidesk_platform::SharedInputInjector for MacOSSharingInjector {
    fn arm_session(&self, session: InputSessionId, duration: Duration) -> bool {
        MacOSSharingInjector::arm_session(self, session, duration)
    }
    fn renew_deadline(&self, duration: Duration) -> bool {
        MacOSSharingInjector::renew_deadline(self, duration)
    }
    fn lease_expired(&self) -> bool {
        MacOSSharingInjector::lease_expired(self)
    }
    fn enqueue_batch(
        &self,
        session: InputSessionId,
        display: &DisplayDescriptor,
        state_epoch: u64,
        events: &[SharedInputEvent],
    ) -> PlatformResult<()> {
        MacOSSharingInjector::enqueue_batch(self, session, display, state_epoch, events)
    }
    fn error_notification(&self) -> Arc<Notify> {
        MacOSSharingInjector::error_notification(self)
    }
    fn take_error(&self) -> Option<PlatformError> {
        MacOSSharingInjector::take_error(self)
    }
    fn release_all(&self) -> PlatformResult<()> {
        MacOSSharingInjector::release_all(self)
    }
    fn release_all_nowait(&self) -> PlatformResult<()> {
        MacOSSharingInjector::release_all_nowait(self)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn pending(state: &MailboxState, event: SharedInputEvent) -> PendingInput {
        PendingInput {
            enqueued_at: Instant::now(),
            generation: state.generation,
            session: state.session.unwrap(),
            display: display(),
            event,
            remaining_events: vec![],
            state_epoch: None,
            reply: None,
        }
    }
    fn armed_mailbox() -> MailboxState {
        MailboxState {
            session: Some(InputSessionId::new()),
            ..Default::default()
        }
    }
    fn batch(state: &MailboxState, epoch: u64, events: Vec<SharedInputEvent>) -> PendingInput {
        let mut input = pending(state, events[0].clone());
        input.remaining_events = events[1..].to_vec();
        input.state_epoch = Some(epoch);
        input
    }
    #[test]
    fn diagnostics_use_bounded_recent_waits_and_lifetime_counts() {
        let mut metrics = InjectionMetrics::default();
        for value in 1..=100 {
            metrics.observe_wait(Duration::from_micros(value));
        }
        metrics.accepted(3, 2);
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.enqueued_batches, 1);
        assert_eq!(snapshot.enqueued_events, 3);
        assert_eq!(snapshot.coalesced_events, 2);
        assert_eq!(snapshot.executed_events, 0);
        assert_eq!(snapshot.queue_wait_p95_us, 95);
        assert_eq!(snapshot.queue_wait_p99_us, 99);
        assert_eq!(snapshot.queue_wait_max_us, 100);
        for _ in 0..WAIT_SAMPLE_CAPACITY {
            metrics.observe_wait(Duration::from_micros(7));
        }
        let snapshot = metrics.snapshot();
        assert_eq!(snapshot.queue_wait_samples, 612);
        assert_eq!(snapshot.queue_wait_recent_samples, 512);
        assert_eq!(snapshot.queue_wait_max_us, 100);
        assert_eq!(snapshot.queue_wait_p95_us, 7);
        assert_eq!(snapshot.queue_wait_p99_us, 7);
    }
    #[test]
    fn batch_motion_coalescing_respects_epoch_and_state_boundaries() {
        let mut state = armed_mailbox();
        let motion = |x| SharedInputEvent::Motion { x, y: 1.0 };
        state.push(batch(&state, 1, vec![motion(1.0)])).unwrap();
        state
            .push(batch(&state, 1, vec![motion(2.0), motion(3.0)]))
            .unwrap();
        assert_eq!(state.inputs.len(), 1);
        assert!(matches!(
            state.inputs[0].event,
            SharedInputEvent::Motion { x: 3.0, .. }
        ));
        state.push(batch(&state, 2, vec![motion(4.0)])).unwrap();
        state
            .push(batch(
                &state,
                2,
                vec![
                    motion(5.0),
                    SharedInputEvent::Button {
                        button: 0,
                        pressed: true,
                    },
                ],
            ))
            .unwrap();
        state.push(batch(&state, 2, vec![motion(6.0)])).unwrap();
        assert_eq!(state.inputs.len(), 4);
        assert_eq!(state.inputs[2].event_count(), 2);
        assert!(!state.inputs[2].pure_motion());
        let counters = state.metrics.snapshot();
        assert_eq!(counters.enqueued_batches, 5);
        assert_eq!(counters.enqueued_events, 7);
        assert_eq!(counters.coalesced_events, 2);
        assert_eq!(counters.executed_events, 0);
    }
    #[test]
    fn invalid_batch_never_commits_valid_anchor_prefix() {
        let mut state = armed_mailbox();
        let input = batch(
            &state,
            1,
            vec![
                SharedInputEvent::Motion { x: 1.0, y: 1.0 },
                SharedInputEvent::Motion {
                    x: f64::NAN,
                    y: 1.0,
                },
            ],
        );
        assert!(state.push(input).is_err());
        assert!(state.inputs.is_empty());
        assert!(state.release_pending);
        assert!(state.session.is_none());
    }
    #[test]
    fn batch_capacity_counts_events_and_fails_without_partial_commit() {
        let mut state = armed_mailbox();
        for _ in 0..MAILBOX_CAPACITY - 1 {
            state
                .push(pending(
                    &state,
                    SharedInputEvent::Scroll { dx: 0.0, dy: 1.0 },
                ))
                .unwrap();
        }
        let input = batch(
            &state,
            1,
            vec![
                SharedInputEvent::Motion { x: 1.0, y: 1.0 },
                SharedInputEvent::Button {
                    button: 0,
                    pressed: true,
                },
            ],
        );
        assert!(state.push(input).is_err());
        assert!(state.inputs.is_empty());
        assert!(state.release_pending);
        assert_eq!(state.error, Some(PlatformError::InputInjectionFailed));
    }
    #[test]
    fn batch_anchor_down_drag_posts_in_order_and_release_rejects_popped_batch() {
        // Quartz event creation can block under WindowServer load. This test
        // checks ordering and explicit revocation; TTL has separate tests.
        let mut native = NativeInjector::new().unwrap();
        let mailbox = Mailbox::default();
        let deadline = Deadline::new();
        let mut state = armed_mailbox();
        let session = state.session.unwrap();
        assert!(deadline.arm(session, Duration::from_secs(1)));
        deadline
            .until
            .store(u64::MAX, std::sync::atomic::Ordering::Release);
        state
            .push(batch(
                &state,
                1,
                vec![
                    SharedInputEvent::Motion { x: 1.0, y: 1.0 },
                    SharedInputEvent::Button {
                        button: 0,
                        pressed: true,
                    },
                ],
            ))
            .unwrap();
        state
            .push(batch(
                &state,
                1,
                vec![SharedInputEvent::Motion { x: 2.0, y: 2.0 }],
            ))
            .unwrap();
        let first = state.inputs.pop_front().unwrap();
        let drag = state.inputs.pop_front().unwrap();
        *mailbox.state.lock().unwrap() = state;
        {
            let _execution = mailbox.execution.lock().unwrap();
            inject_batch(&mailbox, &deadline, &mut native, &first, true).unwrap();
            inject_batch(&mailbox, &deadline, &mut native, &drag, true).unwrap();
        }
        {
            let state = mailbox.state.lock().unwrap();
            assert_eq!(state.metrics.counters.executed_events, 3);
            assert_eq!(state.metrics.counters.queue_wait_samples, 3);
        }
        let kinds: Vec<_> = native
            .posted
            .borrow()
            .iter()
            .map(|event| event.get_type() as u32)
            .collect();
        assert_eq!(
            kinds,
            vec![
                CGEventType::MouseMoved as u32,
                CGEventType::LeftMouseDown as u32,
                CGEventType::LeftMouseDragged as u32
            ]
        );
        {
            let _execution = mailbox.execution.lock().unwrap();
            mailbox.state.lock().unwrap().invalidate();
            native.release_all().unwrap();
            let count = native.posted.borrow().len();
            assert!(inject_batch(&mailbox, &deadline, &mut native, &first, true).is_err());
            assert_eq!(native.posted.borrow().len(), count);
            assert!(native.buttons.is_empty());
        }
    }
    #[test]
    fn mailbox_coalesces_motion_but_preserves_button_and_scroll_boundaries() {
        let mut state = armed_mailbox();
        for x in 0..10_000 {
            state
                .push(pending(
                    &state,
                    SharedInputEvent::Motion {
                        x: x as f64,
                        y: 0.0,
                    },
                ))
                .unwrap();
        }
        assert_eq!(state.inputs.len(), 1);
        state
            .push(pending(
                &state,
                SharedInputEvent::Button {
                    button: 0,
                    pressed: true,
                },
            ))
            .unwrap();
        state
            .push(pending(&state, SharedInputEvent::Motion { x: 3.0, y: 1.0 }))
            .unwrap();
        state
            .push(pending(
                &state,
                SharedInputEvent::Scroll { dx: 0.0, dy: 1.0 },
            ))
            .unwrap();
        state
            .push(pending(&state, SharedInputEvent::Motion { x: 4.0, y: 1.0 }))
            .unwrap();
        assert_eq!(state.inputs.len(), 5);
        assert_eq!(
            state.inputs[0].event,
            SharedInputEvent::Motion { x: 9999.0, y: 0.0 }
        );
        assert!(matches!(
            state.inputs[1].event,
            SharedInputEvent::Button { pressed: true, .. }
        ));
        assert!(matches!(
            state.inputs[3].event,
            SharedInputEvent::Scroll { .. }
        ));
    }
    #[test]
    fn mailbox_does_not_coalesce_display_or_completion_boundaries() {
        let mut state = armed_mailbox();
        let motion = SharedInputEvent::Motion { x: 1.0, y: 2.0 };
        state.push(pending(&state, motion.clone())).unwrap();
        let mut other = pending(&state, motion.clone());
        other.display.id = "other".into();
        state.push(other).unwrap();
        let (tx, _rx) = mpsc::sync_channel(1);
        let mut synchronous = pending(&state, motion.clone());
        synchronous.reply = Some(tx);
        state.push(synchronous).unwrap();
        state.push(pending(&state, motion)).unwrap();
        assert_eq!(state.inputs.len(), 4);
    }
    #[test]
    fn same_session_rearm_rejects_popped_work_and_cancels_queued_work() {
        let mut state = armed_mailbox();
        let session = state.session;
        let popped = pending(&state, SharedInputEvent::Motion { x: 1.0, y: 2.0 });
        let (tx, rx) = mpsc::sync_channel(1);
        let mut queued = pending(
            &state,
            SharedInputEvent::Button {
                button: 0,
                pressed: true,
            },
        );
        queued.reply = Some(tx);
        state.push(queued).unwrap();
        state.invalidate();
        state.session = session;
        assert!(!state.accepts(&popped));
        assert!(state.inputs.is_empty());
        assert!(state.release_pending);
        assert_eq!(
            rx.try_recv().unwrap(),
            Err(PlatformError::InputInjectionDisabled)
        );
        assert!(state.accepts(&pending(
            &state,
            SharedInputEvent::Motion { x: 2.0, y: 3.0 }
        )));
    }
    #[test]
    fn mailbox_overflow_releases_state_instead_of_dropping_input_edges() {
        let mut state = armed_mailbox();
        for _ in 0..MAILBOX_CAPACITY {
            state
                .push(pending(
                    &state,
                    SharedInputEvent::Scroll { dx: 1.0, dy: 0.0 },
                ))
                .unwrap();
        }
        let old = pending(
            &state,
            SharedInputEvent::Button {
                button: 0,
                pressed: false,
            },
        );
        let extra = pending(
            &state,
            SharedInputEvent::Button {
                button: 0,
                pressed: false,
            },
        );
        assert_eq!(state.push(extra), Err(PlatformError::InputInjectionFailed));
        assert!(state.inputs.is_empty());
        assert_eq!(state.session, None);
        assert_eq!(state.error, Some(PlatformError::InputInjectionFailed));
        assert!(state.release_pending);
        assert!(!state.accepts(&old));
    }
    #[test]
    fn lease_expiry_clears_mailbox_and_retires_session_before_rearm() {
        let mut state = armed_mailbox();
        let session = state.session.unwrap();
        let deadline = Deadline::new();
        assert!(deadline.arm(session, Duration::from_millis(1500)));
        let popped = pending(&state, SharedInputEvent::Motion { x: 1.0, y: 2.0 });
        state
            .push(pending(
                &state,
                SharedInputEvent::Button {
                    button: 0,
                    pressed: true,
                },
            ))
            .unwrap();
        deadline
            .until
            .store(0, std::sync::atomic::Ordering::Release);
        assert!(state.expire(&deadline));
        assert!(!state.expire(&deadline));
        assert!(state.inputs.is_empty());
        assert!(!state.accepts(&popped));
        assert!(state.release_pending);
        assert!(!deadline.arm(session, Duration::from_millis(1500)));
        assert!(!deadline.renew(Duration::from_millis(1500)));
    }
    #[test]
    fn fault_retains_first_cause_and_rejects_input_until_explicit_rearm() {
        let mut state = armed_mailbox();
        let old = pending(&state, SharedInputEvent::Motion { x: 1.0, y: 2.0 });
        state.fail(PlatformError::InputPermissionRequired);
        state.fail(PlatformError::InputInjectionFailed);
        assert_eq!(
            state.error.take(),
            Some(PlatformError::InputPermissionRequired)
        );
        assert!(!state.accepts(&old));
        assert!(state.release_pending);
    }
    #[test]
    fn modifier_sides_and_aggregate_fallback() {
        assert_eq!(modifier_keycodes((1 << 17) | 4), BTreeSet::from([60]));
        assert_eq!(
            modifier_keycodes((1 << 18) | (1 << 20)),
            BTreeSet::from([55, 59])
        );
        assert!(modifier_keycodes(0).is_empty());
    }
    #[test]
    fn side_buttons_use_other_native_events() {
        assert_eq!(
            button_type(4, true) as u32,
            CGEventType::OtherMouseDown as u32
        );
        assert_eq!(
            button_type(4, false) as u32,
            CGEventType::OtherMouseUp as u32
        );
    }
    fn display() -> DisplayDescriptor {
        DisplayDescriptor {
            id: "test".into(),
            frame_x: -900.0,
            frame_y: -100.0,
            width: 800,
            height: 600,
            scale_factor: 2.0,
            rotation_degrees: 90,
            is_primary: false,
        }
    }
    #[test]
    fn native_mouse_coordinates_drag_double_click_and_side_button_ledger() {
        let mut n = NativeInjector::new().unwrap();
        let d = display();
        n.inject(&d, &SharedInputEvent::Motion { x: 100.0, y: 200.0 })
            .unwrap();
        n.inject(
            &d,
            &SharedInputEvent::Button {
                button: 0,
                pressed: true,
            },
        )
        .unwrap();
        let point = n.posted.borrow().last().unwrap().location();
        assert_eq!((point.x, point.y), (-800.0, 100.0));
        n.inject(&d, &SharedInputEvent::Motion { x: 101.0, y: 201.0 })
            .unwrap();
        assert_eq!(
            n.posted.borrow().last().unwrap().get_type() as u32,
            CGEventType::LeftMouseDragged as u32
        );
        n.inject(
            &d,
            &SharedInputEvent::Button {
                button: 0,
                pressed: false,
            },
        )
        .unwrap();
        n.inject(
            &d,
            &SharedInputEvent::Button {
                button: 0,
                pressed: true,
            },
        )
        .unwrap();
        assert_eq!(
            n.posted
                .borrow()
                .last()
                .unwrap()
                .get_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE),
            2
        );
        n.inject(
            &d,
            &SharedInputEvent::Button {
                button: 4,
                pressed: true,
            },
        )
        .unwrap();
        assert_eq!(
            n.posted
                .borrow()
                .last()
                .unwrap()
                .get_integer_value_field(EventField::MOUSE_EVENT_BUTTON_NUMBER),
            4
        );
        assert!(n.posted.borrow().iter().all(|event| {
            event.get_integer_value_field(EventField::EVENT_SOURCE_USER_DATA)
                == SYNTHETIC_SHARING_EVENT_MARKER
        }));
        n.release_all().unwrap();
        assert!(n.buttons.is_empty());
        assert!(n.keys.is_empty());
        assert!(n.modifier_keys.is_empty());
    }
    #[test]
    fn scroll_marker_allows_downstream_smoothing_to_retag_events() {
        let mut n = NativeInjector::new().unwrap();
        n.inject(&display(), &SharedInputEvent::Scroll { dx: 0.0, dy: -10.0 })
            .unwrap();
        let events = n.posted.borrow();
        let event = events.last().unwrap();
        assert_eq!(
            event.get_integer_value_field(EventField::EVENT_SOURCE_USER_DATA),
            SYNTHETIC_SHARING_EVENT_MARKER
        );
        // Native scroll processors must be able to mark their derived events
        // without repeatedly processing and swallowing their own output.
        let downstream_marker = 0x1234_5678;
        event.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, downstream_marker);
        assert_eq!(
            event.get_integer_value_field(EventField::EVENT_SOURCE_USER_DATA),
            downstream_marker
        );
        assert_eq!(
            event.get_integer_value_field(EventField::EVENT_SOURCE_UNIX_PROCESS_ID),
            i64::from(std::process::id())
        );
    }

    #[test]
    fn scroll_uses_remote_pointer_location_on_offset_display() {
        let mut n = NativeInjector::new().unwrap();
        let mut d = display();
        d.frame_x = -1920.0;
        d.frame_y = -500.0;
        n.inject(&d, &SharedInputEvent::Motion { x: 40.0, y: 50.0 })
            .unwrap();
        for (dx, dy) in [(0.0, -10.0), (20.0, 0.0), (0.25, -0.5)] {
            n.inject(&d, &SharedInputEvent::Scroll { dx, dy }).unwrap();
            let events = n.posted.borrow();
            let event = events.last().unwrap();
            assert_eq!(event.get_type() as u32, CGEventType::ScrollWheel as u32);
            let location = event.location();
            assert_eq!((location.x, location.y), (-1880.0, -450.0));
        }
    }

    #[test]
    fn native_repeat_flags_fractional_scroll_and_release_all() {
        let mut n = NativeInjector::new().unwrap();
        let d = display();
        let flags = (1 << 17) | 4 | (1 << 20) | 8;
        n.inject(&d, &SharedInputEvent::Motion { x: 4.0, y: 5.0 })
            .unwrap();
        n.inject(
            &d,
            &SharedInputEvent::Key {
                code: 0,
                pressed: true,
                modifiers: flags,
                repeat: true,
            },
        )
        .unwrap();
        assert_eq!(
            n.posted
                .borrow()
                .last()
                .unwrap()
                .get_integer_value_field(EventField::KEYBOARD_EVENT_AUTOREPEAT),
            1
        );
        n.inject(
            &d,
            &SharedInputEvent::Button {
                button: 0,
                pressed: true,
            },
        )
        .unwrap();
        assert_eq!(
            n.posted.borrow().last().unwrap().get_flags().bits() & flags,
            flags
        );
        n.inject(
            &d,
            &SharedInputEvent::Scroll {
                dx: 0.25,
                dy: -1.75,
            },
        )
        .unwrap();
        assert_eq!(
            n.posted
                .borrow()
                .last()
                .unwrap()
                .get_double_value_field(EventField::SCROLL_WHEEL_EVENT_FIXED_POINT_DELTA_AXIS_2),
            0.25
        );
        assert_eq!(
            n.posted
                .borrow()
                .last()
                .unwrap()
                .get_double_value_field(EventField::SCROLL_WHEEL_EVENT_FIXED_POINT_DELTA_AXIS_1),
            -1.75
        );
        n.release_all().unwrap();
        assert!(n.keys.is_empty());
        assert!(n.buttons.is_empty());
        assert_eq!(n.modifiers, 0);
        let events = n.posted.borrow();
        assert!(
            events
                .iter()
                .any(|event| event.get_type() as u32 == CGEventType::KeyUp as u32
                    && event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE) == 0)
        );
        assert!(
            events
                .iter()
                .any(|event| event.get_type() as u32 == CGEventType::LeftMouseUp as u32)
        );
    }
    #[test]
    fn button_before_target_position_is_rejected() {
        let mut n = NativeInjector::new().unwrap();
        assert!(
            n.inject(
                &display(),
                &SharedInputEvent::Button {
                    button: 0,
                    pressed: true
                }
            )
            .is_err()
        );
        assert!(n.posted.borrow().is_empty());
    }
    #[test]
    fn fractional_scroll_accumulates_for_integer_point_consumers() {
        let mut n = NativeInjector::new().unwrap();
        let d = display();
        for _ in 0..4 {
            n.inject(&d, &SharedInputEvent::Scroll { dx: 0.25, dy: 0.0 })
                .unwrap();
        }
        assert_eq!(
            n.posted
                .borrow()
                .iter()
                .map(|e| e
                    .get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_2))
                .sum::<i64>(),
            1
        );
    }
}
