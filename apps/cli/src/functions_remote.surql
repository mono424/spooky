-- ==================================================
-- COMMON REMOTE FUNCTIONS
-- ==================================================

DEFINE FUNCTION fn::query::heartbeat($id: record) {
    UPDATE $id SET lastActiveAt = time::now();
};