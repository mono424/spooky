use smol_str::{self, SmolStr};

#[allow(dead_code)]
enum FieldType {
    Hot,
    Filter,
    Order,
}

#[allow(dead_code)]
struct SpookyFieldSchema {
    table_name: SmolStr,
    field_name: SmolStr,
    value: Vec<u8>,
}

#[allow(dead_code)]
struct SpookyDBSchema {
    hot_fields: Vec<SpookyFieldSchema>,
    filter_fields: Vec<SpookyFieldSchema>,
    order_fields: Vec<SpookyFieldSchema>,
}
