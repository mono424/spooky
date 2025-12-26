-- ==================================================
-- SPOOKY INCANTATION
-- The Registry of active Live Queries (Incantations).
-- ==================================================

DEFINE TABLE _spooky_incantation SCHEMAFULL
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
DEFINE FIELD Tree ON TABLE _spooky_incantation TYPE object
    PERMISSIONS FOR select, create, update WHERE true;

-- For garbage collection (Heartbeat)
DEFINE FIELD LastActiveAt ON TABLE _spooky_incantation TYPE datetime DEFAULT time::now()
    PERMISSIONS FOR select, create, update WHERE true;

-- How long this Incantation stays alive without activity
DEFINE FIELD TTL ON TABLE _spooky_incantation TYPE duration
    PERMISSIONS FOR select, create, update WHERE true;

-- Cleanup Triggers
-- When an incantation dies, clean up its lookup and tail records
DEFINE EVENT _spooky_cascade_delete_lookup ON TABLE _spooky_incantation WHEN $event = "DELETE" THEN {
    DELETE _spooky_incantation_lookup WHERE IncantationId = $before.Id;
    DELETE _spooky_incantation_tail WHERE IncantationId = $before.Id;
};

-- Register with DBSP on creation
DEFINE EVENT _spooky_dbsp_register ON TABLE _spooky_incantation WHEN $event = "CREATE" THEN {
    fn::dbsp::register_query($after.Id, $after.SurrealQL);
};

-- Unregister from DBSP on deletion
DEFINE EVENT _spooky_dbsp_cleanup ON TABLE _spooky_incantation WHEN $event = "DELETE" THEN {
    fn::dbsp::unregister_query($before.Id);
};


-- ==================================================
-- SPOOKY INCANTATION LOOKUP
-- The Reverse Index: Maps Tables -> Incantations
-- ==================================================

DEFINE TABLE _spooky_incantation_lookup SCHEMAFULL
    PERMISSIONS FOR select, create, update WHERE true;

-- Link to the parent Incantation
DEFINE FIELD IncantationId ON TABLE _spooky_incantation_lookup TYPE string
    PERMISSIONS FOR select, create, update WHERE true;

-- The primary table being queried (e.g., 'thread')
DEFINE FIELD Table ON TABLE _spooky_incantation_lookup TYPE string
    PERMISSIONS FOR select, create, update WHERE true;

-- Filter logic used to check if a dirty record matches this query
-- Stored as an object e.g., { clause: "importance >= 3", args: [...] }
DEFINE FIELD Where ON TABLE _spooky_incantation_lookup TYPE object DEFAULT {}
    PERMISSIONS FOR select, create, update WHERE true;

-- Sorting Metadata needed to maintain order
DEFINE FIELD SortFields ON TABLE _spooky_incantation_lookup TYPE option<array<string>>
    PERMISSIONS FOR select, create, update WHERE true;
DEFINE FIELD SortDirections ON TABLE _spooky_incantation_lookup TYPE option<array<string>>
    PERMISSIONS FOR select, create, update WHERE true; -- 'ASC', 'DESC'

-- Indexes for performance
DEFINE INDEX idx_incantation ON TABLE _spooky_incantation_lookup COLUMNS IncantationId;
DEFINE INDEX idx_table ON TABLE _spooky_incantation_lookup COLUMNS Table;


-- ==================================================
-- SPOOKY INCANTATION TAIL
-- Cursor Management for Pagination/Limits
-- ==================================================

DEFINE TABLE _spooky_incantation_tail SCHEMAFULL
    PERMISSIONS FOR select, create, update WHERE true;

-- Link to the parent Incantation
DEFINE FIELD IncantationId ON TABLE _spooky_incantation_tail TYPE string
    PERMISSIONS FOR select, create, update WHERE true;

-- The sort values of the *last* item in the current result set
-- Used to determine if a new record falls inside or outside the LIMIT window.
DEFINE FIELD TailValues ON TABLE _spooky_incantation_tail TYPE array<any>
    PERMISSIONS FOR select, create, update WHERE true;

DEFINE INDEX idx_incantation ON TABLE _spooky_incantation_tail COLUMNS IncantationId UNIQUE;


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
