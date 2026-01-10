use rust_lib_surrealdb::api::client::{SurrealDb, StorageMode};

pub async fn setup_mem() -> SurrealDb {
    SurrealDb::connect(StorageMode::Memory).await.expect("Failed to connect to memory DB")
}
