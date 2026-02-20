import re
import glob

helpers = """
fn insert_record(db: &mut spooky_db_module::db::SpookyDb, table: &str, id: &str, record: ssp::engine::types::SpookyValue) {
    let bytes = spooky_db_module::serialization::from_spooky(&record).unwrap();
    db.apply_mutation(table, spooky_db_module::db::Operation::Create, id, Some(&bytes), None).unwrap();
}

fn delete_record(db: &mut spooky_db_module::db::SpookyDb, table: &str, id: &str) {
    db.apply_mutation(table, spooky_db_module::db::Operation::Delete, id, None, None).unwrap();
}
"""

for path in glob.glob("tests/**/*.rs", recursive=True):
    with open(path, 'r') as f:
        text = f.read()

    original_text = text

    if "SpookyDb::new(" in text or "Database::new(" in text:
        text = text.replace("ssp::engine::circuit::{Circuit, Database, Table}", "ssp::engine::circuit::Circuit;\nuse spooky_db_module::db::SpookyDb")
        text = text.replace("ssp::engine::circuit::{Circuit, Database}", "ssp::engine::circuit::Circuit;\nuse spooky_db_module::db::SpookyDb")
        text = text.replace("ssp::engine::circuit::Database", "spooky_db_module::db::SpookyDb")

        if "fn insert_record" not in text:
            if "use spooky_db_module::db::SpookyDb;" in text:
                text = text.replace("use spooky_db_module::db::SpookyDb;", "use spooky_db_module::db::SpookyDb;\n" + helpers, 1)
            else:
                text = "use spooky_db_module::db::SpookyDb;\n" + helpers + "\n" + text
        
        # fix: let mut db = Database::new() -> let mut db = SpookyDb::new(...)
        text = text.replace("Database::new()", "SpookyDb::new(std::env::temp_dir().join(ulid::Ulid::new().to_string())).unwrap()")
        text = re.sub(r'->\s*Database\s*\{', '-> SpookyDb {', text)

        # Map variable to table name
        var_to_table = {}
        for match in re.finditer(r'let\s+(?:mut\s+)?(\w+)\s*=\s*(?:circuit\.)?db\.tables\.get_mut\("([^"]+)"\)\.unwrap\(\);', text):
            var_to_table[match.group(1)] = match.group(2)
        for match in re.finditer(r'let\s+(?:mut\s+)?(\w+)\s*=\s*(?:circuit\.)?db\.ensure_table\("([^"]+)"\);', text):
            var_to_table[match.group(1)] = match.group(2)
        
        # We need to replace: `table_var.rows.insert(SmolStr::new("id_str"), make_user(...));`
        # and delete `table_var.zset.insert(..., 1);`
        # and `table_var.rows.remove("id_str");` and `table_var.zset.remove(...);`
        
        # We can just use a generic regex to replace `.rows.insert` and`.rows.remove` and ignore `.zset.` 
        # Actually `table_var.zset.insert` might be on the next line. We can just delete all `.zset.insert` and `.zset.remove`!
        text = re.sub(r'(\w+)\.zset\.insert\([^;]+;\s*', '', text)
        text = re.sub(r'(\w+)\.zset\.remove\([^;]+;\s*', '', text)
        
        # Now `.rows.insert`
        # matches: `table_var.rows.insert(KEY, VALUE);`
        
        def replace_insert(m):
            var_name = m.group(1)
            key_expr = m.group(2)
            value_expr = m.group(3)
            
            table_name = var_to_table.get(var_name, var_name.replace("_table", ""))
            
            # Since key_expr is often `SmolStr::new("1")` or `"1"`, 
            # we need to pass a string slice. Let's just pass `&key_expr.to_string()`
            return f'insert_record(&mut db, "{table_name}", &({key_expr}).to_string(), {value_expr});'

        text = re.sub(r'(\w+)\.rows\.insert\(([^,]+),\s*([^;]+)\);', replace_insert, text)

        def replace_remove(m):
            var_name = m.group(1)
            key_expr = m.group(2)
            table_name = var_to_table.get(var_name, var_name.replace("_table", ""))
            return f'delete_record(&mut db, "{table_name}", &({key_expr}).to_string());'
            
        text = re.sub(r'(\w+)\.rows\.remove\(([^;]+)\);', replace_remove, text)

        # In case of `db.tables.get(...)` assertions:
        text = re.sub(r'(?:circuit\.)?db\.tables\.get\("([^"]+)"\)\.unwrap\(\)\.rows\.len\(\)', r'circuit.get_db().table_len("\1")', text)
        text = re.sub(r'(?:circuit\.)?db\.tables\.get\("([^"]+)"\)\.unwrap\(\)\.zset\.contains_key\("([^"]+)"\)', r'(circuit.get_db().get_zset_weight("\1", "\2") > 0)', text)
        text = re.sub(r'(?:circuit\.)?db\.tables\.get\("([^"]+)"\)\.unwrap\(\)\.rows\.contains_key\("([^"]+)"\)', r'(circuit.get_db().get_zset_weight("\1", "\2") > 0)', text)

    if text != original_text:
        with open(path, 'w') as f:
            f.write(text)

