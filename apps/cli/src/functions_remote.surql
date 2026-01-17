-- ==================================================
-- COMMON REMOTE FUNCTIONS
-- ==================================================

DEFINE FUNCTION fn::incantation::heartbeat($id: record) {
    UPDATE $id SET lastActiveAt = time::now();
};