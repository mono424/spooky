import glob
import re

for file in glob.glob("tests/**/*.rs", recursive=True):
    with open(file, 'r') as f:
        content = f.read()
    
    # Imports
    content = content.replace("ssp::engine::circuit::{Circuit, Database, Table}", "ssp::engine::circuit::Circuit\nuse spooky_db_module::db::SpookyDb")
    content = content.replace("ssp::engine::circuit::{Circuit, Database}", "ssp::engine::circuit::Circuit\nuse spooky_db_module::db::SpookyDb")
    content = content.replace("ssp::engine::circuit::Database", "spooky_db_module::db::SpookyDb")
    
    # fn setup_...() -> Database
    content = re.sub(r'->\s*Database\s*\{', '-> SpookyDb {', content)
    
    # Let mut db = Database::new() -> Let mut db = SpookyDb::new(...)
    # If they are tests we will use a tempdir like in predicate.rs tests
    if "SpookyDb::new()" in content or "Database::new()" in content:
        content = content.replace("Database::new()", "SpookyDb::new(std::env::temp_dir().join(ulid::Ulid::new().to_string())).unwrap()")
        content = content.replace("SpookyDb::new()", "SpookyDb::new(std::env::temp_dir().join(ulid::Ulid::new().to_string())).unwrap()")
        
    with open(file, 'w') as f:
        f.write(content)

