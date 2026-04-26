use crate::config::LoadBalanceStrategy;
use crate::messages::RecordUpdate;
use crate::transport::SspInfo;
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Instant;
use tracing::warn;

/// SSP initialization state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SspState {
    /// SSP is bootstrapping from the snapshot proxy
    Bootstrapping,
    /// SSP reported ready, scheduler is replaying missed events
    Replaying,
    /// SSP is fully caught up and receiving live updates
    Ready,
}

/// Pool of connected SSPs with load balancing
pub struct SspPool {
    ssps: HashMap<String, SspInfo>,
    ssp_states: HashMap<String, SspState>,
    message_buffers: HashMap<String, VecDeque<RecordUpdate>>,
    /// Per-SSP snapshot_seq recorded at registration time
    ssp_snapshot_seqs: HashMap<String, u64>,
    /// SSPs that the operator (or an integrity check) has flagged as needing
    /// to re-bootstrap. The next heartbeat from these SSPs returns 409 so
    /// they tear down and re-register against the current frozen snapshot.
    forced_resync: HashSet<String>,
    strategy: LoadBalanceStrategy,
    round_robin_index: usize,
    max_buffer_size: usize,
}

impl SspPool {
    /// Create a new SSP pool with configurable buffer size
    pub fn new(strategy: LoadBalanceStrategy, max_buffer_size: usize) -> Self {
        Self {
            ssps: HashMap::new(),
            ssp_states: HashMap::new(),
            message_buffers: HashMap::new(),
            ssp_snapshot_seqs: HashMap::new(),
            forced_resync: HashSet::new(),
            strategy,
            round_robin_index: 0,
            max_buffer_size,
        }
    }

    /// Flag an SSP for forced re-bootstrap on its next heartbeat. Used by
    /// the integrity-check path when the SSP's circuit hashes disagree with
    /// the scheduler's frozen snapshot — the SSP is told (via 409) to wipe
    /// and re-register rather than continue serving stale state.
    pub fn mark_for_resync(&mut self, ssp_id: &str) {
        self.forced_resync.insert(ssp_id.to_string());
    }

    /// Flag every connected SSP for forced re-bootstrap.
    pub fn mark_all_for_resync(&mut self) -> usize {
        let ids: Vec<String> = self.ssps.keys().cloned().collect();
        for id in &ids {
            self.forced_resync.insert(id.clone());
        }
        ids.len()
    }

    /// Take-and-clear: returns true if this SSP was flagged for forced
    /// resync, removing the flag in the same step.
    pub fn take_resync_flag(&mut self, ssp_id: &str) -> bool {
        self.forced_resync.remove(ssp_id)
    }

    /// Add or update an SSP
    pub fn upsert(&mut self, ssp: SspInfo) {
        self.ssps.insert(ssp.id.clone(), ssp);
    }

    /// Update SSP from heartbeat
    pub fn update_ssp(
        &mut self,
        ssp_id: &str,
        views: usize,
        cpu_usage: Option<f64>,
        memory_usage: Option<f64>,
        version: String,
    ) {
        if let Some(ssp) = self.ssps.get_mut(ssp_id) {
            ssp.last_heartbeat = Instant::now();
            ssp.views = views;
            ssp.cpu_usage = cpu_usage;
            ssp.memory_usage = memory_usage;
            ssp.version = version;
        } else {
            // Add new SSP
            let info = SspInfo {
                id: ssp_id.to_string(),
                url: String::new(), // URL must be set via registration, not heartbeat
                version,
                connected_at: Instant::now(),
                last_heartbeat: Instant::now(),
                query_count: 0,
                views,
                cpu_usage,
                memory_usage,
                env: None,
            };
            self.ssps.insert(ssp_id.to_string(), info);
        }
    }

    /// Buffer a message for an SSP that's not ready yet
    /// Returns true if buffered successfully, false if buffer overflow requires re-bootstrap
    pub fn buffer_message(&mut self, ssp_id: &str, message: RecordUpdate) -> bool {
        // Buffer for SSPs that are bootstrapping or replaying
        match self.ssp_states.get(ssp_id) {
            Some(SspState::Bootstrapping) | Some(SspState::Replaying) => {
                let buffer = self
                    .message_buffers
                    .entry(ssp_id.to_string())
                    .or_insert_with(VecDeque::new);

                // Check if buffer would overflow
                if buffer.len() >= self.max_buffer_size {
                    warn!(
                        "Buffer overflow for SSP '{}' ({} messages). SSP needs to re-bootstrap.",
                        ssp_id,
                        buffer.len()
                    );
                    buffer.clear();
                    return false;
                }

                buffer.push_back(message);
                true
            }
            _ => {
                // SSP is ready or doesn't exist, no buffering needed
                true
            }
        }
    }

    /// Check if SSP has buffer overflow (needs re-bootstrap)
    pub fn has_buffer_overflow(&self, ssp_id: &str) -> bool {
        self.message_buffers
            .get(ssp_id)
            .map(|buf| {
                buf.is_empty()
                    && matches!(
                        self.ssp_states.get(ssp_id),
                        Some(SspState::Bootstrapping) | Some(SspState::Replaying)
                    )
            })
            .unwrap_or(false)
    }

    /// Mark SSP as ready and return any remaining buffered messages
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

    /// Mark SSP as replaying (SSP is ready, scheduler replaying missed events)
    pub fn mark_replaying(&mut self, ssp_id: &str) {
        self.ssp_states
            .insert(ssp_id.to_string(), SspState::Replaying);
    }

    /// Drain buffered messages for an SSP without changing its state
    pub fn drain_buffer(&mut self, ssp_id: &str) -> Vec<RecordUpdate> {
        self.message_buffers
            .get_mut(ssp_id)
            .map(|buf| buf.drain(..).collect())
            .unwrap_or_default()
    }

    /// Record the snapshot_seq at which this SSP was registered
    pub fn set_bootstrap_seq(&mut self, ssp_id: &str, seq: u64) {
        self.ssp_snapshot_seqs.insert(ssp_id.to_string(), seq);
    }

    /// Get the snapshot_seq recorded when this SSP registered
    pub fn get_bootstrap_seq(&self, ssp_id: &str) -> Option<u64> {
        self.ssp_snapshot_seqs.get(ssp_id).copied()
    }

    /// Check if SSP is ready to receive updates
    pub fn is_ready(&self, ssp_id: &str) -> bool {
        matches!(self.ssp_states.get(ssp_id), Some(SspState::Ready))
    }

    /// Get the current state of an SSP
    pub fn get_state(&self, ssp_id: &str) -> Option<&SspState> {
        self.ssp_states.get(ssp_id)
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
        self.ssp_states.remove(ssp_id);
        self.message_buffers.remove(ssp_id);
        self.ssp_snapshot_seqs.remove(ssp_id);
        self.forced_resync.remove(ssp_id);
        self.ssps.remove(ssp_id)
    }

    /// Drop every SSP from the pool and clear all associated buffers/state.
    /// Used when the replica has been restored and SSPs must re-register
    /// against the new state. Returns the count of SSPs removed.
    pub fn clear_all(&mut self) -> usize {
        let count = self.ssps.len();
        self.ssps.clear();
        self.ssp_states.clear();
        self.message_buffers.clear();
        self.ssp_snapshot_seqs.clear();
        self.forced_resync.clear();
        self.round_robin_index = 0;
        count
    }

    /// Get an SSP by ID
    pub fn get(&self, ssp_id: &str) -> Option<&SspInfo> {
        self.ssps.get(ssp_id)
    }

    /// Get all connected SSPs
    pub fn all(&self) -> Vec<&SspInfo> {
        self.ssps.values().collect()
    }

    /// Select the best SSP for a new query based on load balancing strategy.
    /// Only considers SSPs that are in the `Ready` state.
    pub fn select_for_query(&mut self) -> Option<String> {
        let ready_ids: Vec<String> = self
            .ssps
            .keys()
            .filter(|id| matches!(self.ssp_states.get(*id), Some(SspState::Ready)))
            .cloned()
            .collect();

        if ready_ids.is_empty() {
            return None;
        }

        match self.strategy {
            LoadBalanceStrategy::RoundRobin => self.select_round_robin(&ready_ids),
            LoadBalanceStrategy::LeastQueries => self.select_least_queries(&ready_ids),
            LoadBalanceStrategy::LeastLoad => self.select_least_load(&ready_ids),
        }
    }

    /// Select SSP using round-robin
    fn select_round_robin(&mut self, ready_ids: &[String]) -> Option<String> {
        if ready_ids.is_empty() {
            return None;
        }

        let selected = ready_ids[self.round_robin_index % ready_ids.len()].clone();
        self.round_robin_index += 1;
        Some(selected)
    }

    /// Select SSP with fewest queries
    fn select_least_queries(&self, ready_ids: &[String]) -> Option<String> {
        ready_ids
            .iter()
            .filter_map(|id| self.ssps.get(id).map(|info| (id, info)))
            .min_by_key(|(_, info)| info.query_count)
            .map(|(id, _)| id.clone())
    }

    /// Select SSP with least load (CPU + memory)
    fn select_least_load(&self, ready_ids: &[String]) -> Option<String> {
        ready_ids
            .iter()
            .filter_map(|id| self.ssps.get(id).map(|info| (id, info)))
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

    /// Check if any SSP is currently bootstrapping or replaying
    pub fn has_active_bootstrap(&self) -> bool {
        self.ssp_states
            .values()
            .any(|s| matches!(s, SspState::Bootstrapping | SspState::Replaying))
    }
}
