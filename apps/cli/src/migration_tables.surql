DEFINE TABLE IF NOT EXISTS _spooky_migrations SCHEMAFULL PERMISSIONS NONE;
DEFINE FIELD IF NOT EXISTS version ON _spooky_migrations TYPE string;
DEFINE FIELD IF NOT EXISTS name ON _spooky_migrations TYPE string;
DEFINE FIELD IF NOT EXISTS applied_at ON _spooky_migrations TYPE datetime VALUE time::now();
DEFINE FIELD IF NOT EXISTS checksum ON _spooky_migrations TYPE string;
DEFINE INDEX IF NOT EXISTS idx_version ON _spooky_migrations FIELDS version UNIQUE;
