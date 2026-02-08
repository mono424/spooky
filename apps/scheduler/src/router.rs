use crate::config::LoadBalanceStrategy;
use crate::transport::SspInfo;
use std::collections::HashMap;
use std::time::Instant;

/// Pool of connected SSPs with load balancing
pub struct SspPool {
    ssps: HashMap<String, SspInfo>,
    strategy: LoadBalanceStrategy,
    round_robin_index: usize,
}

impl SspPool {
    /// Create a new SSP pool
    pub fn new(strategy: LoadBalanceStrategy) -> Self {
        Self {
            ssps: HashMap::new(),
            strategy,
            round_robin_index: 0,
        }
    }

    /// Add or update an SSP
    pub fn upsert(&mut self, ssp: SspInfo) {
        self.ssps.insert(ssp.id.clone(), ssp);
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
