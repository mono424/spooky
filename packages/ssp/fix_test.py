import re

with open("tests/improved_changes_test.rs", "r") as f:
    text = f.read()

text = text.replace('circuit.db.tables.get("items").unwrap().zset.contains_key("items:1")', 'circuit.get_db().get_zset_weight("items", "items:1") > 0')
text = text.replace('!circuit.db.tables.get("items").unwrap().zset.contains_key("items:1")', 'circuit.get_db().get_zset_weight("items", "items:1") == 0')
text = text.replace('circuit.db.tables.get("items").unwrap().rows.contains_key("items:2")', 'circuit.get_db().get_zset_weight("items", "items:2") > 0')

text = text.replace('let users_table = circuit.db.tables.get("users").unwrap();', '')
text = text.replace('users_table.rows.contains_key("users:1")', 'circuit.get_db().get_zset_weight("users", "users:1") > 0')
text = text.replace('!users_table.rows.contains_key("users:2")', 'circuit.get_db().get_zset_weight("users", "users:2") == 0')

text = re.sub(
    r'let items = circuit\.db\.tables\.get\("items"\)\.unwrap\(\);\n\s*let item = items\.rows\.get\("items:1"\)\.unwrap\(\);\n\s*// SpookyValue comparison - check the counter field\n\s*if let SpookyValue::Object\(map\) = item \{\n\s*if let Some\(SpookyValue::Number\(n\)\) = map\.get\("counter"\) \{\n\s*assert_eq!\(\*n as i32, 3, "Counter should be 3 after all updates"\);\n\s*\}\n\s*\}',
    '''let item = circuit.get_db().get_record_typed("items", "items:1", &["counter"]).unwrap().unwrap();
        if let SpookyValue::Object(map) = item {
            if let Some(counter_val) = map.get("counter") {
                assert_eq!(counter_val, &SpookyValue::from(3), "Counter should be 3 after all updates");
            }
        }''',
    text
)

text = re.sub(
    r'let table = circuit.db.tables.get\("ephemeral"\);\n\s*if let Some\(t\) = table \{\n\s*assert!\(!t\.zset\.contains_key\("ephemeral:1"\), "Record should not exist in ZSet"\);\n\s*\}',
    'assert!(circuit.get_db().get_zset_weight("ephemeral", "ephemeral:1") == 0, "Record should not exist in ZSet");',
    text
)

text = text.replace('circuit.db.tables.get("users").unwrap().rows.len() == 2', 'circuit.get_db().table_len("users") == 2')
text = text.replace('circuit.db.tables.get("posts").unwrap().rows.len() == 1', 'circuit.get_db().table_len("posts") == 1')
text = text.replace('circuit.db.tables.get("comments").unwrap().rows.len() == 1', 'circuit.get_db().table_len("comments") == 1')
text = text.replace('circuit.db.tables.get("books").unwrap().rows.len() == 2', 'circuit.get_db().table_len("books") == 2')

text = re.sub(
    r'let author = circuit\.db\.tables\.get\("authors"\)\.unwrap\(\)\.rows\.get\("authors:1"\)\.unwrap\(\);\n\s*if let SpookyValue::Object\(map\) = author \{\n\s*if let Some\(SpookyValue::Str\(name\)\) = map\.get\("name"\) \{\n\s*assert_eq!\(name\.as_str\(\), "Famous Writer"\);\n\s*\}\n\s*\}',
    '''let author = circuit.get_db().get_record_typed("authors", "authors:1", &["name"]).unwrap().unwrap();
        if let SpookyValue::Object(map) = author {
            if let Some(name) = map.get("name") {
                assert_eq!(name, &SpookyValue::from("Famous Writer"));
            }
        }''',
    text
)

text = text.replace('circuit1.db.tables.get("perf").unwrap().rows.len() == NUM_RECORDS', 'circuit1.get_db().table_len("perf") == NUM_RECORDS')
text = text.replace('circuit2.db.tables.get("perf").unwrap().rows.len() == NUM_RECORDS', 'circuit2.get_db().table_len("perf") == NUM_RECORDS')

text = re.sub(
    r'let table = circuit\.db\.tables\.get\("cycle"\);\n\s*if let Some\(t\) = table \{\n\s*assert!\(!t\.zset\.contains_key\("cycle:1"\)\);\n\s*\}',
    'assert!(circuit.get_db().get_zset_weight("cycle", "cycle:1") == 0);',
    text
)

text = text.replace('circuit.db.tables.get("large").unwrap().rows.len(),\n            BATCH_SIZE', 'circuit.get_db().table_len("large"),\n            BATCH_SIZE')

text = text.replace('circuit.db.tables.get("unicode").unwrap().rows.len() == 3', 'circuit.get_db().table_len("unicode") == 3')
text = text.replace('circuit.db.tables.get("unicode").unwrap().rows.len()', 'circuit.get_db().table_len("unicode")')

with open("tests/improved_changes_test.rs", "w") as f:
    f.write(text)

