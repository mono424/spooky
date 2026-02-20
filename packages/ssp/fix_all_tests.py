import glob
import re

for file in glob.glob("tests/**/*.rs", recursive=True):
    with open(file, 'r') as f:
        content = f.read()

    # edge_test.rs missing import
    if "SpookyDb::new(" in content and "use spooky_db_module::db::SpookyDb;" not in content:
        content = content.replace("use ssp::engine::circuit::Circuit;", "use ssp::engine::circuit::Circuit;\nuse spooky_db_module::db::SpookyDb;")
    
    # replace db.tables.get_mut("table").unwrap() with db directly since SpookyDb mutates itself
    content = re.sub(r'let (\w+)_table\s*=\s*db\.tables\.get_mut\("([^"]+)"\)\.unwrap\(\);\s*', r'', content)
    
    # replace table.rows.insert(SmolStr::new("id"), make_thread("id", "author", "title"));
    # with table.apply_mutation("table", Operation::Create, "id", data, None)
    # Wait, we can't easily do apply_mutation if the tests expect a direct insert.
    # Actually, if we're setting up the db with SpookyDb, maybe we can use apply_mutation for everything.
    # But it requires serialized bytes. 
    # Or we can just keep the python script simple.
    
    # Instead, let's just make a bulk_load logic? No, these tests insert one by one.
    
    # It might be easier to use `apply_mutation` with `some serialized data`.
    # Let's see how db setup is done:
    # `let mut user_data = FastMap::default();` ... `user_table.rows.insert(...)`
    # Let's replace the `user_table.rows.insert` and `user_table.zset.insert` with `apply_mutation` or `bulk_load`.
    
    pass

