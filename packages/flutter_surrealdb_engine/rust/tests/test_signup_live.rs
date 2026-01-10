use rust_lib_surrealdb::api::client::{SurrealDb, StorageMode};

#[tokio::test]
async fn test_live_signup_flow() {
    println!("Connecting to live server...");
    let db = SurrealDb::connect(StorageMode::Remote { 
        url: "ws://127.0.0.1:8000/rpc".to_string() 
    }).await.expect("Failed to connect to remote DB");

    // 1. Signup directly
    let timestamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis();
    let username = format!("rust_signup_user_{}", timestamp);
    let password = "rust_password";
    
    println!("Attempting signup for user: {}", username);
    
    // V3 Format with access/scope
    let signup_creds = format!(r#"{{
        "ns": "main",
        "db": "main",
        "access": "account", 
        "scope": "account",
        "username": "{}",
        "password": "{}"
    }}"#, username, password);

    let token_res = db.signup(signup_creds).await;
    match token_res {
        Ok(token) => {
            println!("✅ Signup success! Token: {}", token);
            assert!(!token.is_empty());
            
            // 2. Verify token works via authenticate
            let auth_res = db.authenticate(token).await;
            assert!(auth_res.is_ok(), "Failed to authenticate with signup token");
            println!("✅ Authentication verified.");
        },
        Err(e) => panic!("❌ Signup failed: {:?}", e),
    }
}
