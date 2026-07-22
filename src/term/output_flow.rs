//! PTY 输出流控：每个 tab 同时只允许一个有限大小的 chunk 在浏览器中解析。

use std::collections::{HashMap, VecDeque};

pub const OUTPUT_CHUNK_BYTES: usize = 64 * 1024;
pub const OUTPUT_HIGH_WATER_BYTES: usize = 512 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub struct OutputChunk {
    pub tab_id: u32,
    pub seq: u64,
    pub data: Vec<u8>,
}

#[derive(Default)]
struct TabOutput {
    pending: VecDeque<u8>,
    in_flight: Option<InFlight>,
    closing: bool,
}

struct InFlight {
    seq: u64,
    data: Vec<u8>,
}

pub struct OutputFlow {
    tabs: HashMap<u32, TabOutput>,
    next_seq: u64,
    pending_bytes: usize,
    chunk_bytes: usize,
    high_water_bytes: usize,
}

impl Default for OutputFlow {
    fn default() -> Self {
        Self::new(OUTPUT_CHUNK_BYTES, OUTPUT_HIGH_WATER_BYTES)
    }
}

impl OutputFlow {
    pub fn new(chunk_bytes: usize, high_water_bytes: usize) -> Self {
        assert!(chunk_bytes > 0);
        assert!(high_water_bytes > 0);
        Self {
            tabs: HashMap::new(),
            next_seq: 1,
            pending_bytes: 0,
            chunk_bytes,
            high_water_bytes,
        }
    }

    pub fn push(&mut self, tab_id: u32, data: Vec<u8>) {
        if data.is_empty() {
            return;
        }
        self.pending_bytes = self.pending_bytes.saturating_add(data.len());
        self.tabs.entry(tab_id).or_default().pending.extend(data);
    }

    pub fn take_ready(&mut self) -> Vec<OutputChunk> {
        let mut ready = Vec::new();
        for (&tab_id, output) in &mut self.tabs {
            if output.in_flight.is_some() || output.pending.is_empty() {
                continue;
            }

            let len = output.pending.len().min(self.chunk_bytes);
            let data: Vec<u8> = output.pending.drain(..len).collect();
            self.pending_bytes = self.pending_bytes.saturating_sub(data.len());

            let seq = self.next_seq;
            self.next_seq = self.next_seq.wrapping_add(1).max(1);
            output.in_flight = Some(InFlight {
                seq,
                data: data.clone(),
            });
            ready.push(OutputChunk { tab_id, seq, data });
        }
        ready
    }

    pub fn acknowledge(&mut self, tab_id: u32, seq: u64) -> bool {
        let Some(output) = self.tabs.get_mut(&tab_id) else {
            return false;
        };
        if output.in_flight.as_ref().map(|chunk| chunk.seq) != Some(seq) {
            return false;
        }
        output.in_flight = None;
        true
    }

    pub fn retry(&mut self, tab_id: u32, seq: u64) -> bool {
        let Some(output) = self.tabs.get_mut(&tab_id) else {
            return false;
        };
        if output.in_flight.as_ref().map(|chunk| chunk.seq) != Some(seq) {
            return false;
        }

        let in_flight = output.in_flight.take().expect("sequence checked above");
        self.pending_bytes = self.pending_bytes.saturating_add(in_flight.data.len());
        for byte in in_flight.data.into_iter().rev() {
            output.pending.push_front(byte);
        }
        true
    }

    pub fn mark_closing(&mut self, tab_id: u32) {
        self.tabs.entry(tab_id).or_default().closing = true;
    }

    pub fn take_drained_closing(&mut self) -> Vec<u32> {
        let drained: Vec<u32> = self
            .tabs
            .iter()
            .filter_map(|(&tab_id, output)| {
                (output.closing && output.pending.is_empty() && output.in_flight.is_none())
                    .then_some(tab_id)
            })
            .collect();
        for tab_id in &drained {
            self.tabs.remove(tab_id);
        }
        drained
    }

    pub fn remove(&mut self, tab_id: u32) {
        if let Some(output) = self.tabs.remove(&tab_id) {
            self.pending_bytes = self.pending_bytes.saturating_sub(output.pending.len());
        }
    }

    pub fn is_saturated(&self) -> bool {
        self.pending_bytes >= self.high_water_bytes
    }

    #[cfg(test)]
    fn pending_bytes(&self) -> usize {
        self.pending_bytes
    }
}

#[cfg(test)]
mod tests {
    use super::OutputFlow;

    #[test]
    fn waits_for_matching_ack_before_releasing_more_output() {
        let mut flow = OutputFlow::new(4, 8);
        flow.push(7, b"abcdefghij".to_vec());

        let first = flow.take_ready();
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].tab_id, 7);
        assert_eq!(first[0].data, b"abcd");
        assert!(flow.take_ready().is_empty());

        assert!(!flow.acknowledge(7, first[0].seq + 1));
        assert!(flow.take_ready().is_empty());

        assert!(flow.acknowledge(7, first[0].seq));
        assert_eq!(flow.take_ready()[0].data, b"efgh");
    }

    #[test]
    fn preserves_order_and_reports_pending_high_water() {
        let mut flow = OutputFlow::new(3, 5);
        flow.push(2, b"ab".to_vec());
        flow.push(2, b"cdef".to_vec());

        assert!(flow.is_saturated());
        let first = flow.take_ready();
        assert_eq!(first[0].data, b"abc");
        assert!(!flow.is_saturated());

        flow.acknowledge(2, first[0].seq);
        let second = flow.take_ready();
        assert_eq!(second[0].data, b"def");
    }

    #[test]
    fn remove_discards_pending_and_in_flight_state() {
        let mut flow = OutputFlow::new(4, 8);
        flow.push(9, b"abcdef".to_vec());
        let in_flight = flow.take_ready();

        flow.remove(9);

        assert!(!flow.acknowledge(9, in_flight[0].seq));
        assert!(flow.take_ready().is_empty());
        assert_eq!(flow.pending_bytes(), 0);
    }

    #[test]
    fn failed_delivery_requeues_the_in_flight_chunk_at_the_front() {
        let mut flow = OutputFlow::new(4, 8);
        flow.push(3, b"abcdefgh".to_vec());
        let first = flow.take_ready().remove(0);

        assert!(flow.retry(3, first.seq));

        let retried = flow.take_ready().remove(0);
        assert_eq!(retried.data, b"abcd");
        assert_ne!(retried.seq, first.seq);
        assert!(flow.acknowledge(3, retried.seq));
        assert_eq!(flow.take_ready()[0].data, b"efgh");
    }

    #[test]
    fn closing_tab_is_only_released_after_pending_and_in_flight_output_drains() {
        let mut flow = OutputFlow::new(4, 8);
        flow.push(5, b"abcdef".to_vec());
        flow.mark_closing(5);
        let first = flow.take_ready().remove(0);

        assert!(flow.take_drained_closing().is_empty());
        assert!(flow.acknowledge(5, first.seq));
        let second = flow.take_ready().remove(0);
        assert!(flow.take_drained_closing().is_empty());
        assert!(flow.acknowledge(5, second.seq));
        assert_eq!(flow.take_drained_closing(), vec![5]);
    }
}
