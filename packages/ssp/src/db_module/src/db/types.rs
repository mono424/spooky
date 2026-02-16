use smol_str::{self, SmolStr};

enum FieldType {
    Hot,
    Filter,
    Order,
}

struct SpookyFieldSchema {
    tableName: SmolStr,
    fieldName: SmolStr,
    value: Vec<u8>,
}

struct SpookyDBSchema {
    hotFields: Vec<SpookyFieldSchema>,
    filterFields: Vec<SpookyFieldSchema>,
    orderFields: Vec<SpookyFieldSchema>,
}
