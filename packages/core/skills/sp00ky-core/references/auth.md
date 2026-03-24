# Authentication Reference

## AuthService API

The `AuthService` is available on `Sp00kyClient` as `client.auth`.

### Properties

| Property | Type | Description |
|----------|------|-------------|
| `token` | `string \| null` | Current auth token |
| `currentUser` | `any \| null` | Current authenticated user record |
| `isAuthenticated` | `boolean` | Whether a user is authenticated |
| `isLoading` | `boolean` | Whether auth state is being validated |

### Methods

#### `signUp(accessName, params)`

Create a new account using the named access definition from the schema.

```typescript
await client.auth.signUp('user_access', {
  email: 'alice@example.com',
  password: 'secret123',
  name: 'Alice',
});
```

The access definition in the schema determines what parameters are required:

```typescript
access: {
  user_access: {
    signup: {
      params: {
        email: { type: 'string', optional: false },
        password: { type: 'string', optional: false },
        name: { type: 'string', optional: true },
      },
    },
    signIn: {
      params: {
        email: { type: 'string', optional: false },
        password: { type: 'string', optional: false },
      },
    },
  },
}
```

Parameters are fully type-safe — TypeScript will infer the correct types from the schema.

#### `signIn(accessName, params)`

Authenticate with existing credentials.

```typescript
await client.auth.signIn('user_access', {
  email: 'alice@example.com',
  password: 'secret123',
});
```

#### `signOut()`

Sign out, clear the session token, and notify subscribers.

```typescript
await client.auth.signOut();
```

#### `subscribe(callback)`

Subscribe to auth state changes. The callback fires immediately with the current state, then on every change.

```typescript
const unsub = client.auth.subscribe((userId: string | null) => {
  if (userId) {
    console.log('Logged in as', userId);
  } else {
    console.log('Not authenticated');
  }
});

// Cleanup
unsub();
```

#### `check(accessToken?)`

Manually re-validate the current session. Called automatically during `init()`.

```typescript
await client.auth.check();
// Or with a specific token:
await client.auth.check('eyJhbGciOiJIUzI1NiJ9...');
```
