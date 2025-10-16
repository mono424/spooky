# Solid.js Thread Application

A Solid.js application with authentication and thread management, using SurrealDB for data storage.

## Features

- **Authentication**: Username/password based authentication
- **Thread Management**: Create, view, and comment on threads
- **Real-time Data**: Reactive data loading with Solid.js
- **Local + Remote Storage**: Uses SurrealDB with both local WASM and remote sync capabilities

## Getting Started

### Prerequisites

- Node.js 18+
- pnpm

### Installation

1. Install dependencies:

```bash
pnpm install
```

2. Start the development server:

```bash
pnpm dev
```

3. Open your browser to `http://localhost:3000`

## Usage

1. **Sign Up/Sign In**: Create an account or sign in with existing credentials
2. **View Threads**: Browse all threads on the home page
3. **Create Thread**: Click "New Thread" to create a new discussion thread
4. **Comment**: Click on any thread to view details and add comments

## Database

The app uses SurrealDB with:

- **Local Storage**: IndexedDB for offline-first functionality
- **Remote Sync**: Optional remote SurrealDB server for data synchronization
- **Schema**: Defined in `/example/schema/schema.surql`

## Configuration

To enable remote sync, update the database configuration in `src/lib/db.ts`:

```typescript
const dbConfig: SyncedDbConfig = {
  localDbName: "thread-app-local",
  storageStrategy: "indexeddb",
  namespace: "main",
  database: "thread_app",
  remoteUrl: "http://localhost:8000", // Uncomment for remote sync
  token: "your-auth-token-here", // Add your auth token
};
```

## Project Structure

```
src/
├── components/          # UI Components
│   ├── AuthDialog.tsx   # Authentication modal
│   ├── ThreadList.tsx   # Thread listing
│   ├── ThreadDetail.tsx # Thread view with comments
│   └── CommentForm.tsx  # Comment creation form
├── lib/
│   ├── auth.tsx         # Authentication context
│   └── db.ts            # Database configuration
├── routes/              # Route components
└── styles/              # Global styles
```
