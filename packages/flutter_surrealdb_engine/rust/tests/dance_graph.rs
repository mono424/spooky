mod common;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use surrealdb::types::RecordId;

// Dance classes table name
const DANCE: &str = "dance";
// Students table name
const STUDENT: &str = "student";

const SCHEMA_QUERY: &str = "
    DEFINE TABLE dance SCHEMAFULL;
    DEFINE FIELD name ON TABLE dance TYPE string;
    DEFINE FIELD created_at ON TABLE dance TYPE string;

    DEFINE TABLE student SCHEMAFULL;
    DEFINE FIELD name ON TABLE student TYPE string;
    DEFINE FIELD created_at ON TABLE student TYPE string;
    
    // Using the generic compatible schema pattern
    DEFINE FIELD classes ON TABLE student TYPE array;
    DEFINE FIELD classes.* ON TABLE student TYPE any VALUE type::record($value);
";

// Dance class table schema - simulating generic client using Strings for IDs
#[derive(Debug, Serialize, Deserialize)]
struct DanceClass {
    id: Option<String>,
    name: String,
    created_at: String,
}

// Student table schema
#[derive(Debug, Serialize)]
struct Student {
    id: Option<String>, 
    name: String,
    classes: Vec<String>, // Sending generic strings "dance:id"
    created_at: String,
}

// Student model with full class details for FETCH results
// Here we deserialize the RESULT, where relations are expanded significantly or kept as links.
// If FETCH used, `classes` will be objects.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct DanceClassResult {
    id: RecordId, // SurrealDB returns RecordId object in Rust client usually, generic JSON client sees string?
                  // The wrapper returns JSON string. serde_json::from_str -> Value -> Struct.
                  // If we use RecordId here, serde support keys?
                  // Let's use Value for flexibility in test assertion first.
    name: String, 
}

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct StudentResult {
    id: RecordId,
    name: String,
    classes: Vec<DanceClassResult>,
    created_at: String,
}

#[tokio::test]
async fn test_dance_graph_fetch() -> anyhow::Result<()> {
    // 1. Connect to In-Memory DB
    let db = common::setup_mem().await;
    db.use_db("test".to_string(), "test".to_string()).await?;

    // 2. Load Schema
    db.query(SCHEMA_QUERY.to_string(), None).await?;

    // 3. Create a dance class
    // Pass simple ID string "dc101", so DB generates "dance:dc101"
    let dc1_simple_id = "dc101".to_string();
    let dance_cls = DanceClass {
        id: Some(dc1_simple_id),
        name: "Introduction to Dancing".to_owned(),
        created_at: "2024-01-01T12:00:00Z".to_string(),
    };
    
    println!("Dance Class Serialized: {}", serde_json::to_string(&dance_cls)?);

    let created_dance_str = db.create(
        DANCE.to_string(), 
        Some(serde_json::to_string(&dance_cls)?)
    ).await?;
    
    println!("Created Dance Class JSON: {}", created_dance_str);

    // 4. Create a student with link string
    // We construct the string "dance:dc101" manually for the link
    let dc1_link = format!("{}:{}", DANCE, "dc101");
    let jane_str = format!("{}:{}", STUDENT, "jane");
    let student = Student {
        id: Some(jane_str),
        name: "Jane Doe".to_owned(),
        classes: vec![dc1_link], // Pass string link!
        created_at: "2024-01-01T12:00:00Z".to_string(),
    };

    let created_student_str = db.create(
        STUDENT.to_string(),
        Some(serde_json::to_string(&student)?)
    ).await?;
    println!("Created Student JSON: {}", created_student_str);
    
    // Check if empty (indicates failure in wrapper usually)
    if created_student_str.trim() == "" || created_student_str == "[]" {
        panic!("Student creation returned empty response");
    }

    // 5. Run a query to retrieve students and full class info (FETCH)
    let results = db.query(
        format!("SELECT * FROM {} FETCH classes", STUDENT), 
        None
    ).await?;

    // 6. Extract result and deserialize
    println!("Fetch JSON Result: {}", results);
    
    let parsed_results: Value = serde_json::from_str(&results)?;
    // Query result structure: [[ { ... } ]]
    let students_json = &parsed_results[0]; 
    println!("Students Array: {}", students_json);

    // Manual assertion via Value to avoid struct complexity issues during debugging
    let student_0 = &students_json[0];
    assert_eq!(student_0["name"], "Jane Doe");
    
    // Check classes array
    let classes_arr = student_0["classes"].as_array().expect("Classes should be array");
    assert_eq!(classes_arr.len(), 1);
    
    let class_0 = &classes_arr[0];
    // If FETCH worked, class_0 is an object, not a string
    assert!(class_0.is_object(), "Fetched class should be an object");
    assert_eq!(class_0["name"], "Introduction to Dancing");

    Ok(())
}
