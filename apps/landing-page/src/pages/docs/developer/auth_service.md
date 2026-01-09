---
layout: ../../../layouts/DocsLayout.astro
title: Auth Service
---

# AuthService

The `AuthService` is the gatekeeper of the Spooky Core module. It manages user sessions, persists authentication tokens, and ensures the `RemoteDatabaseService` is correctly authenticated.

## 📦 Responsibility

- **Session Management**: Tracks whether a user is logged in or out.
- **Token Persistence**: Saves and retrieves auth tokens from secure storage (e.g., `localStorage`).
- **Remote Authentication**: Configures the `surrealdb.js` client with the user's credentials.

## 🏗️ Architecture & Boundaries

As a "Black Box" service, `AuthService` has strict boundaries:

- **Inputs**:
  - User credentials (via `authenticate()`).
  - Logout requests (via `invalidate()`).
- **Outputs**:
  - Global authentication state (implicitly affecting `RemoteDatabaseService`).
- **Dependencies**:
  - `RemoteDatabaseService`: The only service `AuthService` interacts with directly.

## 🔄 Input/Output Reference

| Method                | Type      | Description                                                     |
| :-------------------- | :-------- | :-------------------------------------------------------------- |
| `authenticate(token)` | **Input** | Authenticates the user with the given token. Returns a Promise. |
| `invalidate()`        | **Input** | Logs the user out and clears the session.                       |
| `isAuthenticated()`   | **Query** | Returns the current authentication status (boolean).            |

## 🔑 Key Workflows

### Authentication Flow

1. User provides a token (e.g., from a login form).
2. `AuthService.authenticate(token)` is called.
3. The token is sent to the Remote DB's `signin` or `authenticate` endpoint.
4. On success, the token is persisted to `localStorage`.
5. The `RemoteDatabaseService` client is updated with the new session.

### Logout Flow

1. `AuthService.invalidate()` is called.
2. The session is cleared from the Remote DB.
3. The token is removed from `localStorage`.
4. `RemoteDatabaseService` reverts to a guest/anonymous state.

## ⚠️ Internal Logic

- **Persistence**: The service automatically attempts to restore the session from `localStorage` on initialization.
- **Error Handling**: If authentication fails (e.g., expired token), the service will automatically clear the invalid session to prevent loop states.
