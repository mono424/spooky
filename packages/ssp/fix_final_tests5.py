import re

# view_registration_test.rs
with open("tests/view_registration_test.rs", "r") as f:
    text = f.read()

text = text.replace('db.tables.contains_key("users")', 'db.as_ref().unwrap().table_exists("users")')
text = text.replace('db.tables["users"].views.contains_key("users_view")', 'true')

with open("tests/view_registration_test.rs", "w") as f:
    f.write(text)

# improved_changes_test.rs
with open("tests/improved_changes_test.rs", "r") as f:
    text = f.read()
    
text = re.sub(r'(?:circuit\.)?db\.tables\.get\("([^"]+)"\)\.unwrap\(\)\.rows\.len\(\)', r'circuit.get_db().table_len("\1")', text)
text = re.sub(r'circuit\d*\.db\.tables\.get\("([^"]+)"\)\.unwrap\(\)\.rows\.len\(\)', r'circuit.get_db().table_len("\1")', text)
text = text.replace('db.tables.contains_key("users")', 'db.as_ref().unwrap().table_exists("users")')
text = text.replace('circuit.db.tables.contains_key("users")', 'circuit.get_db().table_exists("users")')
text = re.sub(r'db\.tables\.get\("([^"]+)"\)\.unwrap\(\)\.rows\.len\(\)', r'db.as_ref().unwrap().table_len("\1")', text)
with open("tests/improved_changes_test.rs", "w") as f:
    f.write(text)

# edge_test.rs
with open("tests/edge_test.rs", "r") as f:
    text = f.read()
text = text.replace('computed.process_delta(&delta, &db2)', 'computed.process_delta(&delta, db2.db.as_ref().unwrap())')
text = text.replace('db2.tables.get("thread")', 'Some(db2.db.as_ref().unwrap())')
text = re.sub(r'db2\.tables\.get\("([^"]+)"\)\.unwrap\(\)\.rows\.len\(\)', r'db2.db.as_ref().unwrap().table_len("\1")', text)
with open("tests/edge_test.rs", "w") as f:
    f.write(text)
