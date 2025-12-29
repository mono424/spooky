-- ==================================================
-- SPOOKY INCANTATION
-- The Registry of active Live Queries (Incantations).
-- ==================================================

DEFINE TABLE _spooky_incantation SCHEMALESS
    PERMISSIONS FOR select, create, update WHERE $_spooky_client_id = $clientId;

-- The raw query string (for re-hydration/debugging)
-- The raw query string (for re-hydration/debugging)
DEFINE FIELD surrealQL ON TABLE _spooky_incantation TYPE option<string>
    PERMISSIONS FOR select, create, update WHERE $_spooky_client_id = $clientId;

-- The raw query string (for re-hydration/debugging)
DEFINE FIELD clientId ON TABLE _spooky_incantation TYPE option<string>
    PERMISSIONS FOR select, create, update WHERE $_spooky_client_id = $clientId;

-- The current XOR sum of all results in this query
DEFINE FIELD hash ON TABLE _spooky_incantation TYPE option<string>
    PERMISSIONS FOR select, create, update WHERE $_spooky_client_id = $clientId;

-- The Radix Tree of Result IDs for efficient sync
DEFINE FIELD tree ON TABLE _spooky_incantation TYPE any
    PERMISSIONS FOR select, create, update WHERE $_spooky_client_id = $clientId;

-- For garbage collection (Heartbeat)
DEFINE FIELD lastActiveAt ON TABLE _spooky_incantation TYPE datetime DEFAULT time::now()
    PERMISSIONS FOR select, create, update WHERE $_spooky_client_id = $clientId;

-- How long this Incantation stays alive without activity
DEFINE FIELD ttl ON TABLE _spooky_incantation TYPE duration
    PERMISSIONS FOR select, create, update WHERE $_spooky_client_id = $clientId;

