import re

with open("tests/improved_changes_test.rs", "r") as f:
    text = f.read()

text = re.sub(r'users_table\.rows\.contains_key\("([^"]+)"\)', r'(circuit.get_db().get_zset_weight("users", "\1") > 0)', text)
text = re.sub(r'items\.rows\.contains_key\("([^"]+)"\)', r'(circuit.get_db().get_zset_weight("items", "\1") > 0)', text)

with open("tests/improved_changes_test.rs", "w") as f:
    f.write(text)

