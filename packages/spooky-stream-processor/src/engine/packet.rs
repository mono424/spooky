use rkyv::{Archive, Deserialize, Serialize};

#[derive(Archive, Deserialize, Serialize, Debug)]
#[archive(check_bytes)]
pub struct IngestPacket {
    pub table: String,
    pub op: String,
    pub id: String,
    pub record_json: String, // Kept simple to avoid recursive type issues with rkyv for now.
    pub hash: String,
}

#[derive(Archive, Deserialize, Serialize, Debug)]
#[archive(check_bytes)]
pub struct IngestBatch {
    pub packets: Vec<IngestPacket>,
}
