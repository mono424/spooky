-- ==================================================
-- 1. SPOOKY DATA HASH
-- The "Shadow Graph" tracking the state of every record.
-- ==================================================

DEFINE TABLE spooky_data_hash SCHEMAFULL
    PERMISSIONS FULL; -- In prod, you might restrict write access to server-side only

-- The actual record being tracked (e.g., comment:abc, thread:123)
DEFINE FIELD RecordId ON TABLE spooky_data_hash TYPE record;

-- H_intrinsic: BLAKE3 hash of the record's own scalar fields
DEFINE FIELD IntrinsicHash ON TABLE spooky_data_hash TYPE bytes;

-- H_composition: XOR sum of all dependent children's TotalHashes
DEFINE FIELD CompositionHash ON TABLE spooky_data_hash TYPE bytes;

-- H_total: Intrinsic XOR Composition
DEFINE FIELD TotalHash ON TABLE spooky_data_hash TYPE bytes;

-- Fast lookup by the original record ID
DEFINE INDEX idx_record_id ON TABLE spooky_data_hash COLUMNS RecordId UNIQUE;


-- ==================================================
-- 2. SPOOKY INCANTATION
-- The Registry of active Live Queries (Incantations).
-- ==================================================

DEFINE TABLE spooky_incantation SCHEMAFULL
    PERMISSIONS FULL;

-- The unique hash ID of the query + params
DEFINE FIELD Id ON TABLE spooky_incantation TYPE string;

-- The raw query string (for re-hydration/debugging)
DEFINE FIELD SurrealQL ON TABLE spooky_incantation TYPE string;

-- The current XOR sum of all results in this query
DEFINE FIELD Hash ON TABLE spooky_incantation TYPE bytes;

-- For garbage collection (Heartbeat)
DEFINE FIELD LastActiveAt ON TABLE spooky_incantation TYPE datetime DEFAULT time::now();

-- How long this Incantation stays alive without activity
DEFINE FIELD TTL ON TABLE spooky_incantation TYPE duration;

-- Cleanup Triggers
-- When an incantation dies, clean up its lookup and tail records
DEFINE EVENT cascade_delete_lookup ON TABLE spooky_incantation WHEN $event = "DELETE" THEN {
    DELETE spooky_incantation_lookup WHERE IncantationId = $before.Id;
    DELETE spooky_incantation_tail WHERE IncantationId = $before.Id;
};


-- ==================================================
-- 3. SPOOKY INCANTATION LOOKUP
-- The Reverse Index: Maps Tables -> Incantations
-- ==================================================

DEFINE TABLE spooky_incantation_lookup SCHEMAFULL
    PERMISSIONS FULL;

-- Link to the parent Incantation
DEFINE FIELD IncantationId ON TABLE spooky_incantation_lookup TYPE string;

-- The primary table being queried (e.g., 'thread')
DEFINE FIELD Table ON TABLE spooky_incantation_lookup TYPE string;

-- Filter logic used to check if a dirty record matches this query
-- Stored as an object e.g., { clause: "importance >= 3", args: [...] }
DEFINE FIELD Where ON TABLE spooky_incantation_lookup TYPE object;

-- Sorting Metadata needed to maintain order
DEFINE FIELD SortFields ON TABLE spooky_incantation_lookup TYPE array<string>;
DEFINE FIELD SortDirections ON TABLE spooky_incantation_lookup TYPE array<string>; -- 'ASC', 'DESC'

-- Indexes for performance
DEFINE INDEX idx_incantation ON TABLE spooky_incantation_lookup COLUMNS IncantationId;
DEFINE INDEX idx_table ON TABLE spooky_incantation_lookup COLUMNS Table;


-- ==================================================
-- 4. SPOOKY INCANTATION TAIL
-- Cursor Management for Pagination/Limits
-- ==================================================

DEFINE TABLE spooky_incantation_tail SCHEMAFULL
    PERMISSIONS FULL;

-- Link to the parent Incantation
DEFINE FIELD IncantationId ON TABLE spooky_incantation_tail TYPE string;

-- The sort values of the *last* item in the current result set
-- Used to determine if a new record falls inside or outside the LIMIT window.
DEFINE FIELD TailValues ON TABLE spooky_incantation_tail TYPE array<any>;

DEFINE INDEX idx_incantation ON TABLE spooky_incantation_tail COLUMNS IncantationId UNIQUE;


-- ==================================================
-- 5. SPOOKY RELATIONSHIP
-- The Graph Schema: Defines 'Bubble Up' vs 'Cascade Down'
-- ==================================================

DEFINE TABLE spooky_relationship SCHEMAFULL
    PERMISSIONS FULL;

DEFINE FIELD ParentTable ON TABLE spooky_relationship TYPE string;
DEFINE FIELD ChildTable ON TABLE spooky_relationship TYPE string;

-- The field on the Child that holds the Parent's ID
DEFINE FIELD ChildField ON TABLE spooky_relationship TYPE string;

-- The Logic Flow: 'COMPOSITION' (Bubble Up) or 'REFERENCE' (Cascade Down)
DEFINE FIELD Type ON TABLE spooky_relationship TYPE string 
    ASSERT $value INSIDE ['COMPOSITION', 'REFERENCE'];

-- Enforce unique definition per relationship path
DEFINE INDEX idx_rel_unique ON TABLE spooky_relationship COLUMNS ParentTable, ChildTable, ChildField UNIQUE;
