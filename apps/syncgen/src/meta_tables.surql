-- ==================================================
-- SPOOKY INCANTATION
-- The Registry of active Live Queries (Incantations).
-- ==================================================

DEFINE TABLE _spooky_incantation SCHEMALESS
    PERMISSIONS FOR select, create, update WHERE true;

-- The unique hash ID of the query + params
DEFINE FIELD Id ON TABLE _spooky_incantation TYPE string
    PERMISSIONS FOR select, create, update WHERE true;

-- The raw query string (for re-hydration/debugging)
DEFINE FIELD SurrealQL ON TABLE _spooky_incantation TYPE option<string>
    PERMISSIONS FOR select, create, update WHERE true;

-- The raw query string (for re-hydration/debugging)
DEFINE FIELD ClientId ON TABLE _spooky_incantation TYPE option<string>
    PERMISSIONS FOR select, create, update WHERE true;

-- The current XOR sum of all results in this query
DEFINE FIELD Hash ON TABLE _spooky_incantation TYPE string
    PERMISSIONS FOR select, create, update WHERE true;

-- The Radix Tree of Result IDs for efficient sync
DEFINE FIELD Tree ON TABLE _spooky_incantation TYPE any
    PERMISSIONS FOR select, create, update WHERE true;

-- For garbage collection (Heartbeat)
DEFINE FIELD LastActiveAt ON TABLE _spooky_incantation TYPE datetime DEFAULT time::now()
    PERMISSIONS FOR select, create, update WHERE true;

-- How long this Incantation stays alive without activity
DEFINE FIELD TTL ON TABLE _spooky_incantation TYPE duration
    PERMISSIONS FOR select, create, update WHERE true;

-- ==================================================
-- SPOOKY RELATIONSHIP
-- The Graph Schema: Defines 'Bubble Up' vs 'Cascade Down'
-- ==================================================

DEFINE TABLE _spooky_relationship SCHEMAFULL
    PERMISSIONS FOR select, create, update WHERE true;

DEFINE FIELD ParentTable ON TABLE _spooky_relationship TYPE string
    PERMISSIONS FOR select, create, update WHERE true;
DEFINE FIELD ChildTable ON TABLE _spooky_relationship TYPE string
    PERMISSIONS FOR select, create, update WHERE true;

-- The field on the Child that holds the Parent's ID
DEFINE FIELD ChildField ON TABLE _spooky_relationship TYPE string
    PERMISSIONS FOR select, create, update WHERE true;

-- The Logic Flow: 'COMPOSITION' (Bubble Up) or 'REFERENCE' (Cascade Down)
DEFINE FIELD Type ON TABLE _spooky_relationship TYPE string 
    ASSERT $value INSIDE ['COMPOSITION', 'REFERENCE']
    PERMISSIONS FOR select, create, update WHERE true;

-- Enforce unique definition per relationship path
DEFINE INDEX idx_rel_unique ON TABLE _spooky_relationship COLUMNS ParentTable, ChildTable, ChildField UNIQUE;
