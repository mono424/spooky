use rust_lib_surrealdb::api::client::{SurrealDb, StorageMode};

#[tokio::test]
async fn test_live_signin_flow() {
    println!("Connecting to live server...");
    let db = SurrealDb::connect(StorageMode::Remote { 
        url: "ws://127.0.0.1:8000/rpc".to_string() 
    }).await.expect("Failed to connect to remote DB");

    // 1. Signin as Root to create user
    println!("Signing in as root...");
    let root_creds = r#"{"user":"root","pass":"root"}"#;
    db.signin(root_creds.to_string()).await.expect("Failed to signin as root");

    db.use_db("main".to_string(), "main".to_string()).await.expect("Failed to use db");

    // 2. Create User manually
    let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
    let username = format!("rust_signin_user_{}", timestamp);
    let password = "rust_password";
    
    println!("Creating user: {}", username);
    
    // Using explicit vars for safety and to test var binding
    let create_sql = "CREATE ONLY user SET username = $username, password = crypto::argon2::generate($password);";
    let vars_json = format!(r#"{{"username": "{}", "password": "{}"}}"#, username, password);
    
    let create_res = db.query(create_sql.to_string(), Some(vars_json)).await;
    match create_res {
        Ok(res) => println!("User created: {}", res),
        Err(e) => panic!("Failed to create user: {:?}", e),
    }

    // 3. Signin as new user
    println!("Attempting signin as new user...");
    
    // V3 Format with access/scope
    let signin_creds = format!(r#"{{
        "ns": "main",
        "db": "main",
        "access": "account", 
        "scope": "account",
        "username": "{}",
        "password": "{}"
    }}"#, username, password);

    let token_res = db.signin(signin_creds).await;
    match token_res {
        Ok(token) => {
            println!("✅ Signin success! Token length: {}", token.len());
            assert!(!token.is_empty());
            
            // 4. Verify token works
            let auth_res = db.authenticate(token).await;
            assert!(auth_res.is_ok(), "Failed to authenticate with new token");
            println!("✅ Authentication verified.");
        },
        Err(e) => panic!("❌ Signin failed: {:?}", e),
    }
}
