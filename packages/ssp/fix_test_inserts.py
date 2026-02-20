import re
import glob

def fix_file(path):
    with open(path, 'r') as f:
        text = f.read()

    # Add helper functions if not present
    if "fn insert_record" not in text:
        helpers = """
fn insert_record(db: &mut spooky_db_module::db::SpookyDb, table: &str, id: &str, record: SpookyValue) {
    let bytes = spooky_db_module::serialization::from_spooky(&record).unwrap();
    db.apply_mutation(table, spooky_db_module::db::Operation::Create, id, Some(&bytes), None).unwrap();
}

fn delete_record(db: &mut spooky_db_module::db::SpookyDb, table: &str, id: &str) {
    db.apply_mutation(table, spooky_db_module::db::Operation::Delete, id, None, None).unwrap();
}
"""
        # Insert after imports
        if "use spooky_db_module::db::SpookyDb;" in text:
            text = text.replace("use spooky_db_module::db::SpookyDb;", "use spooky_db_module::db::SpookyDb;" + helpers)
        else:
            text = text.replace("use ssp::engine::", "use spooky_db_module::db::SpookyDb;\nuse ssp::engine::", 1)
            text = text.replace("use spooky_db_module::db::SpookyDb;", "use spooky_db_module::db::SpookyDb;" + helpers)

    # user_table.rows.insert(SmolStr::new("1"), make_user("1", "Alice Updated"));
    # user_table.zset.insert(make_zset_key("user", "1"), 1);
    
    # regex for table_name variable: let ___table = db.tables.get_mut("...").unwrap();
    text = re.sub(r'let (\w+)_table = db\.tables\.get_mut\("([^"]+)"\)\.unwrap\(\);', r'// \1_table mapping for \2', text)
    text = re.sub(r'let (\w+)_table = db\.ensure_table\("([^"]+)"\);', r'// \1_table mapping for \2', text)

    # For edge_test.rs and delta_edge_test.rs where tables are managed directly,
    # find all .rows.insert and replace with insert_record
    
    text = re.sub(r'(\w+)_table\.rows\.insert\(SmolStr::new\("([^"]+)"\),\s*([^;]+)\);?\n\s*\w+_table\.zset\.insert\([^;]+\);?', 
                  r'insert_record(&mut db, "\1", "\2", \3);', text)
                  
    text = re.sub(r'(\w+)_table\.rows\.insert\(SmolStr::new\("([^"]+)"\),\s*([^;]+)\);?', 
                  r'insert_record(&mut db, "\1", "\2", \3);', text)

    text = re.sub(r'(\w+)_table\.rows\.insert\(SmolStr::new\(&(\w+)\.to_string\(\)\),\s*([^;]+)\);?\n\s*\w+_table\.zset\.insert\([^;]+\);?', 
                  r'insert_record(&mut db, "\1", &\2.to_string(), \3);', text)

    text = re.sub(r'(\w+)_table\.rows\.remove\("([^"]+)"\);?\n\s*\w+_table\.zset\.remove\([^;]+\);?', 
                  r'delete_record(&mut db, "\1", "\2");', text)

    # If Db backend passed mutably: insert_record(&mut db, ...)
    # Sometimes it's `db`, sometimes `circuit.db` ... Wait! In circuit tests `db` is inside `circuit` or passed directly.
    # We will adjust specific failed assertions.
    
    # In edge_test.rs:
    text = text.replace("table.rows.insert", "insert_record(&mut db, ")
    
    with open(path, 'w') as f:
        f.write(text)

fix_file("tests/delta_edge_test.rs")
fix_file("tests/edge_test.rs")
fix_file("tests/update_fix_test.rs")
fix_file("tests/view_registration_test.rs")
fix_file("tests/improved_changes_test.rs")
