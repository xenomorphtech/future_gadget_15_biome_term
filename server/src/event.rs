use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone)]
pub struct Event {
    pub seq: u64,
    pub timestamp_ms: u64,
    pub data: Vec<u8>,
}

pub struct EventLog {
    events: Vec<Event>,
    next_seq: u64,
}

impl EventLog {
    pub fn new() -> Self {
        EventLog {
            events: Vec::new(),
            next_seq: 1,
        }
    }

    pub fn push(&mut self, data: Vec<u8>) -> u64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        self.events.push(Event { seq, timestamp_ms, data });
        seq
    }

    /// Returns events with seq > after_seq. after=0 returns all events.
    pub fn since(&self, after_seq: u64) -> Vec<Event> {
        self.events
            .iter()
            .filter(|e| e.seq > after_seq)
            .cloned()
            .collect()
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
