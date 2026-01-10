#!/bin/bash
# Generate unique username
USERNAME="curl_user_$(date +%s)"
echo "Attempting Signup via HTTP for user: $USERNAME"

# Current Schema has password READ permissions = TRUE (from last step)
# This is ideal for testing.

curl -v -X POST http://127.0.0.1:8000/signup \
  -H "Accept: application/json" \
  -H "Content-Type: application/json" \
  -H "surreal-ns: main" \
  -H "surreal-db: main" \
  -d "{
    \"ns\": \"main\",
    \"db\": \"main\",
    \"access\": \"account\",
    \"username\": \"$USERNAME\",
    \"password\": \"secure_password\"
  }"
echo ""
