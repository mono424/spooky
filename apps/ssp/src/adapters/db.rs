use ssp_node::{Db, DbError};

use crate::SharedDb;

/// `ssp_node::Db` over the surrealdb SDK's HTTP engine.
pub struct SurrealSdkDb {
    db: SharedDb,
}

impl SurrealSdkDb {
    pub fn new(db: SharedDb) -> Self {
        Self { db }
    }

    /// Run one statement and flatten the FIRST result to plain JSON.
    ///
    /// `into_json_value()` flattens RecordId/Datetime to plain strings and
    /// unwraps SurrealDB's tagged Value enum into ordinary JSON —
    /// `serde_json::to_value(&val)` would emit the tagged shape
    /// `{"Object": {...}}`, which breaks `.get("tables")`-style access.
    /// Shared by the `Db` impl and `BootstrapSource::Direct`.
    pub async fn flatten_first(
        db: &SharedDb,
        surql: &str,
    ) -> anyhow::Result<serde_json::Value> {
        use anyhow::Context;
        let handle = db.handle();
        let mut response = handle
            .query(surql)
            .await
            .inspect_err(|e| db.note_error(&e.to_string()))
            .with_context(|| format!("Query failed: {}", surql))?;
        let val: surrealdb::types::Value = response
            .take(0)
            .context("Failed to parse query response")?;
        Ok(val.into_json_value())
    }
}

#[async_trait::async_trait]
impl Db for SurrealSdkDb {
    async fn query(
        &self,
        surql: &str,
        binds: &[(&str, serde_json::Value)],
    ) -> Result<Vec<serde_json::Value>, DbError> {
        let handle = self.db.handle();
        let mut q = handle.query(surql);
        for (name, value) in binds {
            q = q.bind(((*name).to_string(), value.clone()));
        }
        // Every SSP database call funnels through here, so this is the one
        // place that has to tell the connection its session died — that report
        // is what makes a SurrealDB restart heal on the next failed query
        // instead of at the next refresh tick.
        let mut response = q.await.map_err(|e| {
            let msg = e.to_string();
            self.db.note_error(&msg);
            DbError::Transport(msg)
        })?;

        let n = response.num_statements();
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let val: surrealdb::types::Value = response
                .take(i)
                .map_err(|e| DbError::Query(e.to_string()))?;
            out.push(val.into_json_value());
        }
        Ok(out)
    }

    async fn version(&self) -> Result<String, DbError> {
        self.db
            .handle()
            .version()
            .await
            .map(|v| v.to_string())
            .map_err(|e| {
                let msg = e.to_string();
                self.db.note_error(&msg);
                DbError::Transport(msg)
            })
    }
}
