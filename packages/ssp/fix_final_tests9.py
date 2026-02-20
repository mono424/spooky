import re

# view_registration_test.rs
with open("tests/view_registration_test.rs", "r") as f:
    text = f.read()

text = re.sub(r'user_table\s*\n\s*\.zset\s*\n\s*\.contains_key\("1"\)', r'(circuit.get_db().get_zset_weight("user", "1") > 0)', text)
text = re.sub(r'user_table\.rows\.contains_key\("1"\)', r'(circuit.get_db().get_zset_weight("user", "1") > 0)', text)

with open("tests/view_registration_test.rs", "w") as f:
    f.write(text)

# improved_changes_test.rs
with open("tests/improved_changes_test.rs", "r") as f:
    text = f.read()

text = text.replace('items.rows.contains_key("items:1")', '(circuit.get_db().get_zset_weight("items", "items:1") > 0)')
text = text.replace('items.rows.contains_key("items:2")', '(circuit.get_db().get_zset_weight("items", "items:2") > 0)')

text = text.replace('table.zset.contains_key("ephemeral:1")', '(circuit.get_db().get_zset_weight("ephemeral", "ephemeral:1") > 0)')
text = text.replace('table.zset.contains_key("cycle:1")', '(circuit.get_db().get_zset_weight("cycle", "cycle:1") > 0)')

# Fix the authors:1 reading since I replaced it with `Some(circuit.get_db())`
# Find the exact line
text = text.replace('let author = Some(circuit.get_db()) /* removed user get */;', 
                    'let bytes = circuit.get_db().get_record_typed::<ssp::SpookyValue>("authors", "authors:1").unwrap();\n        let author = bytes;')
with open("tests/improved_changes_test.rs", "w") as f:
    f.write(text)

