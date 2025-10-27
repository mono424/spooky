# Use the official SurrealDB image
FROM surrealdb/surrealdb:latest

# Expose the default SurrealDB port
EXPOSE 8000

# Start SurrealDB with the schema file
# Using memory storage for development - change to file:// for persistence
ENV SURREAL_CAPS_ALLOW_EXPERIMENTAL = "record_references"
CMD ["start", "--log", "trace", "--user", "root", "--pass", "root", "memory"]

