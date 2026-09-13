//! Bounded host-owned event queue.

use std::collections::VecDeque;
use velin_syntax::{DataFootprint, Value};

/// A side-effect-only host event returned by the batch runner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostEvent {
    pub name: String,
    pub values: Vec<Value>,
}

/// Limits for a host-owned event queue. The queue is deliberately outside the
/// VM so an application can choose a capacity and consumption policy without
/// changing execution semantics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostEventQueueLimits {
    pub capacity: usize,
    pub max_values: usize,
    pub max_text_bytes: usize,
}

impl Default for HostEventQueueLimits {
    fn default() -> Self {
        Self {
            capacity: 1_024,
            max_values: 1_000_000,
            max_text_bytes: 64 * 1024 * 1024,
        }
    }
}

/// Failure to enqueue a host event without exceeding the host's backpressure
/// budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostEventQueueError {
    Full,
    ValuesBudget,
    TextBudget,
    InvalidValue(&'static str),
}

impl std::fmt::Display for HostEventQueueError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Full => formatter.write_str("host event queue is full"),
            Self::ValuesBudget => formatter.write_str("host event queue value budget exceeded"),
            Self::TextBudget => formatter.write_str("host event queue text budget exceeded"),
            Self::InvalidValue(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for HostEventQueueError {}

#[derive(Debug)]
struct QueuedHostEvent {
    event: HostEvent,
    footprint: DataFootprint,
}

/// A bounded, FIFO host queue with aggregate payload accounting.
#[derive(Debug)]
pub struct HostEventQueue {
    limits: HostEventQueueLimits,
    events: VecDeque<QueuedHostEvent>,
    values: usize,
    text_bytes: usize,
}

impl HostEventQueue {
    #[must_use]
    pub fn new(limits: HostEventQueueLimits) -> Self {
        Self {
            limits,
            events: VecDeque::with_capacity(limits.capacity.min(64)),
            values: 0,
            text_bytes: 0,
        }
    }

    #[must_use]
    pub const fn limits(&self) -> HostEventQueueLimits {
        self.limits
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.events.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    #[must_use]
    pub fn values(&self) -> usize {
        self.values
    }

    #[must_use]
    pub fn text_bytes(&self) -> usize {
        self.text_bytes
    }

    /// Enqueues an event after validating every value and aggregate payload.
    ///
    /// # Errors
    /// Returns an error when the queue is full, a value is invalid, or the
    /// event would exceed a configured values or text-byte budget.
    pub fn push(&mut self, event: HostEvent) -> Result<(), HostEventQueueError> {
        if self.events.len() >= self.limits.capacity {
            return Err(HostEventQueueError::Full);
        }
        let mut footprint = DataFootprint::default();
        for value in &event.values {
            let metrics = value
                .data_metrics()
                .map_err(HostEventQueueError::InvalidValue)?;
            footprint.values = footprint
                .values
                .checked_add(metrics.footprint.values)
                .ok_or(HostEventQueueError::ValuesBudget)?;
            footprint.text_bytes = footprint
                .text_bytes
                .checked_add(metrics.footprint.text_bytes)
                .ok_or(HostEventQueueError::TextBudget)?;
        }
        let values = self
            .values
            .checked_add(footprint.values)
            .ok_or(HostEventQueueError::ValuesBudget)?;
        if values > self.limits.max_values {
            return Err(HostEventQueueError::ValuesBudget);
        }
        let text_bytes = self
            .text_bytes
            .checked_add(footprint.text_bytes)
            .ok_or(HostEventQueueError::TextBudget)?;
        if text_bytes > self.limits.max_text_bytes {
            return Err(HostEventQueueError::TextBudget);
        }
        self.values = values;
        self.text_bytes = text_bytes;
        self.events.push_back(QueuedHostEvent { event, footprint });
        Ok(())
    }

    pub fn pop_front(&mut self) -> Option<HostEvent> {
        let queued = self.events.pop_front()?;
        self.values -= queued.footprint.values;
        self.text_bytes -= queued.footprint.text_bytes;
        Some(queued.event)
    }
}
