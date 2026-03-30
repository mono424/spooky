DEFINE TABLE IF NOT EXISTS _00_migrations SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD IF NOT EXISTS version ON _00_migrations TYPE string;
DEFINE FIELD IF NOT EXISTS name ON _00_migrations TYPE string;
DEFINE FIELD IF NOT EXISTS applied_at ON _00_migrations TYPE datetime VALUE time::now();
DEFINE FIELD IF NOT EXISTS checksum ON _00_migrations TYPE string;
DEFINE INDEX IF NOT EXISTS idx_version ON _00_migrations FIELDS version UNIQUE;
