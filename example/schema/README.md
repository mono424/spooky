# SurrealDB Schema

This folder contains the database schema for the Thread App.

## Running SurrealDB with Docker

### Option 1: Using Docker Compose (Recommended)

```bash
# Start SurrealDB
docker-compose up -d

# View logs
docker-compose logs -f

# Stop SurrealDB (keeps data)
docker-compose down

# Stop and remove all data
docker-compose down -v
```

If you encounter permission errors, try:

```bash
# Fix volume permissions
docker-compose down -v
docker volume create surrealdb-data
docker-compose up -d
```

### Option 2: Using Docker directly

```bash
# Build the image
docker build -t thread-app-surrealdb .

# Run the container
docker run -d -p 8000:8000 --name surrealdb thread-app-surrealdb

# View logs
docker logs -f surrealdb

# Stop and remove
docker stop surrealdb && docker rm surrealdb
```

## Applying the Schema

After starting SurrealDB, you can apply the schema using the SurrealDB CLI or by importing it:

```bash
# Using the CLI
surreal import --conn http://localhost:8000 --user root --pass root --ns main --db thread_app schema.surql

# Or use the web interface
# Navigate to http://localhost:8000
```

## Connection Details

- **URL**: `http://localhost:8000`
- **Username**: `root`
- **Password**: `root`
- **Namespace**: `main`
- **Database**: `thread_app`

## Schema Files

- `schema.surql` - Main schema definition with tables, fields, and permissions
- `Dockerfile` - Docker configuration for running SurrealDB
- `docker-compose.yml` - Docker Compose configuration for easy deployment
