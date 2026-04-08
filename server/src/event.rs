use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_MAX_EVENTS: usize = 10_000;

#[derive(Clone)]
pub struct Event {
    pub seq: u64,
    pub timestamp_ms: u64,
    pub data: Vec<u8>,
}

pub struct EventLog {
    events: VecDeque<Event>,
    next_seq: u64,
    max_events: usize,
}

impl Default for EventLog {
    fn default() -> Self {
        Self::new()
    }
}

impl EventLog {
    pub fn new() -> Self {
        Self::with_max_events(DEFAULT_MAX_EVENTS)
    }

    pub fn with_max_events(max_events: usize) -> Self {
        EventLog {
            events: VecDeque::new(),
            next_seq: 1,
            max_events,
        }
    }

    pub fn push(&mut self, data: Vec<u8>) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        if self.events.len() >= self.max_events {
            self.events.pop_front();
        }
        self.events.push_back(Event {
            seq,
            timestamp_ms,
            data,
        });
        seq
    }

    /// Returns events with seq > after_seq. after=0 returns all retained events.
    /// If after_seq is older than the oldest retained event, returns all retained events.
    pub fn since(&self, after_seq: u64) -> Vec<Event> {
        if self.events.is_empty() {
            return Vec::new();
        }
        let start = self.events.partition_point(|e| e.seq <= after_seq);
        self.events.iter().skip(start).cloned().collect()
    }

    /// Returns the seq of the oldest retained event, or None if empty.
    pub fn oldest_seq(&self) -> Option<u64> {
        self.events.front().map(|e| e.seq)
    }

    /// Returns the seq of the newest retained event, or None if empty.
    pub fn latest_seq(&self) -> Option<u64> {
        self.events.back().map(|e| e.seq)
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
