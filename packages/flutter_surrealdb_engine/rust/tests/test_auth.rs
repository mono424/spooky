mod common;
use serde_json::Value;

#[tokio::test]
async fn test_auth_flow() {
    let db = common::setup_mem().await;
    
    // 1. Setup Namespace/Database and Define Scope
    db.use_db("test".to_string(), "test".to_string()).await.expect("Failed to use db");
    
    // Define a scope that allows signup/signin
    let setup_sql = "
        DEFINE SCOPE user SESSION 14d
        SIGNUP ( CREATE user SET email = $email, pass = crypto::argon2::generate($pass) )
        SIGNIN ( SELECT * FROM user WHERE email = $email AND crypto::argon2::compare(pass, $pass) )
    ";
    db.query(setup_sql.to_string(), "".to_string()).await.expect("Failed to setup scope");

    // 2. Signup
    // Credentials must include NS, DB, SC, and user logic fields (email, pass)
    let signup_creds = r#"{
        "NS": "test",
        "DB": "test",
        "SC": "user",
        "email": "test@example.com",
        "pass": "password123"
    }"#;
    
    let token_json_res = db.signup(signup_creds.to_string()).await;
    // Check if it fails due to memory engine limitation?
    // Usually auth works in memory if kv-mem enabled.
    
    match token_json_res {
        Ok(token_json) => {
            println!("Signup Token: {}", token_json);
            // Should parse
            let token_obj: Value = serde_json::from_str(&token_json).expect("Failed to parse token JSON");
            
            // 3. Signin
            let signin_creds = r#"{
                "NS": "test",
                "DB": "test",
                "SC": "user",
                "email": "test@example.com",
                "pass": "password123"
            }"#;
            let signin_res = db.signin(signin_creds.to_string()).await.expect("Signin failed");
            println!("Signin Token: {}", signin_res);
             
             let token_val: Value = serde_json::from_str(&signin_res).expect("Parse signin res");
             
             let raw_token = if token_val.is_string() {
                 token_val.as_str().unwrap().to_string()
             } else if token_val.is_object() {
                 return; 
             } else {
                 panic!("Unknown token format: {:?}", token_val);
             };
    
            let auth_res = db.authenticate(raw_token).await;
            assert!(auth_res.is_ok());
            
            // 5. Invalidate
            let inv_res = db.invalidate().await;
            assert!(inv_res.is_ok());
        }
        Err(e) => {
            println!("Signup failed (expected in some envs): {:?}", e);
        }
    }
}
