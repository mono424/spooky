use crate::config::LoadBalanceStrategy;
use crate::messages::RecordUpdate;
use crate::transport::SspInfo;
use std::collections::{HashMap, VecDeque};
use std::time::Instant;
use tracing::warn;

/// SSP initialization state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SspState {
    /// SSP is bootstrapping (not ready to receive live updates)
    Bootstrapping,
    /// SSP is ready and receiving live updates
    Ready,
}

/// Pool of connected SSPs with load balancing
pub struct SspPool {
    ssps: HashMap<String, SspInfo>,
    ssp_states: HashMap<String, SspState>,
    message_buffers: HashMap<String, VecDeque<RecordUpdate>>,
    strategy: LoadBalanceStrategy,
    round_robin_index: usize,
    max_buffer_size: usize,
}

impl SspPool {
    /// Create a new SSP pool
    pub fn new(strategy: LoadBalanceStrategy) -> Self {
        Self {
            ssps: HashMap::new(),
            ssp_states: HashMap::new(),
            message_buffers: HashMap::new(),
            strategy,
            round_robin_index: 0,
            max_buffer_size: 1000, // Max buffered messages per SSP
        }
    }

    /// Add or update an SSP
    pub fn upsert(&mut self, ssp: SspInfo) {
        self.ssps.insert(ssp.id.clone(), ssp);
    }

    /// Update SSP from heartbeat
    pub fn update_ssp(
        &mut self,
        ssp_id: &str,
        active_queries: usize,
        cpu_usage: Option<f64>,
        memory_usage: Option<f64>,
    ) {
        if let Some(ssp) = self.ssps.get_mut(ssp_id) {
            ssp.last_heartbeat = Instant::now();
            ssp.active_jobs = active_queries;
            ssp.cpu_usage = cpu_usage;
            ssp.memory_usage = memory_usage;
        } else {
            // Add new SSP
            let info = SspInfo {
                id: ssp_id.to_string(),
                connected_at: Instant::now(),
                last_heartbeat: Instant::now(),
                query_count: 0,
                active_jobs: active_queries,
                cpu_usage,
                memory_usage,
            };
            self.ssps.insert(ssp_id.to_string(), info);
        }
    }

    /// Buffer a message for an SSP that's not ready yet
    /// Returns true if buffered successfully, false if buffer overflow requires re-bootstrap
    pub fn buffer_message(&mut self, ssp_id: &str, message: RecordUpdate) -> bool {
        // Check if SSP is bootstrapping
        if let Some(SspState::Bootstrapping) = self.ssp_states.get(ssp_id) {
            let buffer = self
                .message_buffers
                .entry(ssp_id.to_string())
                .or_insert_with(VecDeque::new);

            // Check if buffer would overflow
            if buffer.len() >= self.max_buffer_size {
                // Buffer overflow - SSP is too slow or bootstrap is taking too long
                // Clear the buffer and mark as needing re-bootstrap
                warn!(
                    "Buffer overflow for SSP '{}' ({} messages). SSP needs to re-bootstrap.",
                    ssp_id,
                    buffer.len()
                );
                buffer.clear();
                // SSP will need to re-bootstrap to get consistent state
                return false;
            }

            buffer.push_back(message);
            true
        } else {
            // SSP is ready or doesn't exist, no buffering needed
            true
        }
    }

    /// Check if SSP has buffer overflow (needs re-bootstrap)
    pub fn has_buffer_overflow(&self, ssp_id: &str) -> bool {
        self.message_buffers
            .get(ssp_id)
            .map(|buf| {
                buf.is_empty() && self.ssp_states.get(ssp_id) == Some(&SspState::Bootstrapping)
            })
            .unwrap_or(false)
    }

    /// Mark SSP as ready and get buffered messages
    pub fn mark_ready(&mut self, ssp_id: &str) -> Vec<RecordUpdate> {
        self.ssp_states.insert(ssp_id.to_string(), SspState::Ready);

        // Return and clear buffered messages
        self.message_buffers
            .remove(ssp_id)
            .map(|buf| buf.into_iter().collect())
            .unwrap_or_default()
    }

    /// Mark SSP as bootstrapping
    pub fn mark_bootstrapping(&mut self, ssp_id: &str) {
        self.ssp_states
            .insert(ssp_id.to_string(), SspState::Bootstrapping);
    }

    /// Check if SSP is ready to receive updates
    pub fn is_ready(&self, ssp_id: &str) -> bool {
        matches!(self.ssp_states.get(ssp_id), Some(SspState::Ready))
    }

    /// Get buffer size for an SSP
    pub fn buffer_size(&self, ssp_id: &str) -> usize {
        self.message_buffers
            .get(ssp_id)
            .map(|buf| buf.len())
            .unwrap_or(0)
    }

    /// Remove an SSP
    pub fn remove(&mut self, ssp_id: &str) -> Option<SspInfo> {
        self.ssps.remove(ssp_id)
    }

    /// Get an SSP by ID
    pub fn get(&self, ssp_id: &str) -> Option<&SspInfo> {
        self.ssps.get(ssp_id)
    }

    /// Get all connected SSPs
    pub fn all(&self) -> Vec<&SspInfo> {
        self.ssps.values().collect()
    }

    /// Select the best SSP for a new query based on load balancing strategy
    pub fn select_for_query(&mut self) -> Option<String> {
        if self.ssps.is_empty() {
            return None;
        }

        match self.strategy {
            LoadBalanceStrategy::RoundRobin => self.select_round_robin(),
            LoadBalanceStrategy::LeastQueries => self.select_least_queries(),
            LoadBalanceStrategy::LeastLoad => self.select_least_load(),
        }
    }

    /// Select SSP using round-robin
    fn select_round_robin(&mut self) -> Option<String> {
        let ssps: Vec<_> = self.ssps.keys().cloned().collect();
        if ssps.is_empty() {
            return None;
        }

        let selected = ssps[self.round_robin_index % ssps.len()].clone();
        self.round_robin_index += 1;
        Some(selected)
    }

    /// Select SSP with fewest queries
    fn select_least_queries(&self) -> Option<String> {
        self.ssps
            .iter()
            .min_by_key(|(_, info)| info.query_count)
            .map(|(id, _)| id.clone())
    }

    /// Select SSP with least load (CPU + memory)
    fn select_least_load(&self) -> Option<String> {
        self.ssps
            .iter()
            .min_by(|(_, a), (_, b)| {
                let load_a = a.cpu_usage.unwrap_or(0.0) + a.memory_usage.unwrap_or(0.0);
                let load_b = b.cpu_usage.unwrap_or(0.0) + b.memory_usage.unwrap_or(0.0);
                load_a
                    .partial_cmp(&load_b)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(id, _)| id.clone())
    }

    /// Increment query count for an SSP
    pub fn increment_query_count(&mut self, ssp_id: &str) {
        if let Some(ssp) = self.ssps.get_mut(ssp_id) {
            ssp.query_count += 1;
        }
    }

    /// Decrement query count for an SSP
    pub fn decrement_query_count(&mut self, ssp_id: &str) {
        if let Some(ssp) = self.ssps.get_mut(ssp_id) {
            ssp.query_count = ssp.query_count.saturating_sub(1);
        }
    }

    /// Get SSPs that haven't sent a heartbeat within the timeout
    pub fn get_stale_ssps(&self, timeout_ms: u64) -> Vec<String> {
        let now = Instant::now();
        let timeout = std::time::Duration::from_millis(timeout_ms);

        self.ssps
            .iter()
            .filter(|(_, info)| now.duration_since(info.last_heartbeat) > timeout)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Count of connected SSPs
    pub fn count(&self) -> usize {
        self.ssps.len()
    }
}
