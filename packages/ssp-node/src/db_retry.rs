//! Bounded retry for statements that lose an optimistic-concurrency race.
//!
//! SurrealDB runs optimistic (MVCC) transactions: a statement whose write set
//! overlaps a transaction that committed while it was in flight fails with
//! "Transaction conflict: Transaction write conflict. This transaction can be
//! retried" (or, under RocksDB lock pressure, "Transaction conflict: Resource
//! busy"), and a statement after a failed one in the same request echoes "The
//! query was not executed due to a failed transaction". Nothing is wrong and
//! nothing is lost, the statement simply has to run again against the newer
//! snapshot.
//!
//! Every writer on the hot rows (`_00_query`, `_00_list_ref_*`) hits this
//! under load, because they all write the same rows: the edge transaction
//! after an ingest, the per-view metrics, the TTL sweep, client heartbeats.
//! Shared here so each of them retries the same way.

use serde_json::Value;
use tracing::debug;

use crate::ports::{Db, DbError};

/// Is this the transient kind of failure a retry can clear?
pub fn is_write_conflict(e: &DbError) -> bool {
    let msg = match e {
        DbError::Query(m) | DbError::Transport(m) => m.to_ascii_lowercase(),
        DbError::Auth(_) => return false,
    };
    msg.contains("write conflict")
        || msg.contains("transaction conflict")
        || msg.contains("can be retried")
        || msg.contains("resource busy")
        // The per-statement echo of an earlier statement's failure inside one
        // request: retrying the whole request is right, the budget below is
        // what keeps a genuinely bad statement from spinning.
        || msg.contains("failed transaction")
}

/// Attempts for a statement that lost an optimistic-concurrency race. No sleep
/// between tries: a conflict means the *other* transaction already committed,
/// so the retry runs against a settled snapshot (and a sleep here is not
/// portable to the wasm shell, which has no timer inside a request).
pub const CONFLICT_RETRIES: usize = 5;

/// `Db::query` with bounded retries on a write conflict. Any other error, and
/// an exhausted budget, are returned to the caller unchanged.
pub async fn query_retrying(
    db: &dyn Db,
    surql: &str,
    binds: &[(&str, Value)],
) -> Result<Vec<Value>, DbError> {
    let mut attempt = 0usize;
    loop {
        match db.query(surql, binds).await {
            Ok(v) => return Ok(v),
            Err(e) if is_write_conflict(&e) && attempt < CONFLICT_RETRIES => {
                attempt += 1;
                debug!(attempt, error = %e, "write conflict, retrying statement");
            }
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_every_conflict_spelling_seen_in_production() {
        for m in [
            "Transaction conflict: Transaction write conflict. This transaction can be retried",
            "Transaction conflict: Resource busy: . This transaction can be retried",
            "The query was not executed due to a failed transaction",
        ] {
            assert!(is_write_conflict(&DbError::Query(m.into())), "{m}");
        }
    }

    #[test]
    fn does_not_match_a_real_error() {
        assert!(!is_write_conflict(&DbError::Query("Parse error: unexpected token".into())));
        assert!(!is_write_conflict(&DbError::Auth("nope".into())));
    }
}
