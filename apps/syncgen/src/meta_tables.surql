-- ==================================================
-- SPOOKY INCANTATION
-- The Registry of active Live Queries (Incantations).
-- ==================================================

DEFINE TABLE _spooky_incantation SCHEMALESS
    PERMISSIONS FOR select, create, update WHERE true;

-- The raw query string (for re-hydration/debugging)
-- The raw query string (for re-hydration/debugging)
DEFINE FIELD surrealQL ON TABLE _spooky_incantation TYPE option<string>
    PERMISSIONS FOR select, create, update WHERE true;

-- The raw query string (for re-hydration/debugging)
DEFINE FIELD clientId ON TABLE _spooky_incantation TYPE option<string>
    PERMISSIONS FOR select, create, update WHERE true;



-- For garbage collection (Heartbeat)
DEFINE FIELD lastActiveAt ON TABLE _spooky_incantation TYPE datetime DEFAULT time::now()
    PERMISSIONS FOR select, create, update WHERE true;

-- How long this Incantation stays alive without activity
DEFINE FIELD ttl ON TABLE _spooky_incantation TYPE duration
    PERMISSIONS FOR select, create, update WHERE true;
