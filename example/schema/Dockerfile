# Use the official SurrealDB image
FROM surrealdb/surrealdb:latest

# Expose the default SurrealDB port
EXPOSE 8000

# Copy the schema file into the container
COPY schema.surql /schema.surql

# Start SurrealDB with the schema file
# Using memory storage for development - change to file:// for persistence
CMD ["start", "--log", "trace", "--user", "root", "--pass", "root", "memory"]

