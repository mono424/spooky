-- ==================================================
-- COMMON REMOTE FUNCTIONS
-- ==================================================

DEFINE FUNCTION OVERWRITE fn::query::heartbeat($id: record) {
    UPDATE $id SET lastActiveAt = time::now();
};