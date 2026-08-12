-- ==================================================
-- COMMON REMOTE FUNCTIONS
-- ==================================================

-- Prove this session is still watching a SHARED incantation.
--
-- Any subscriber may call it. The gate is `_00_query`'s auth-scoped update
-- rule, not the registrant's `session::id()` — which is the whole point:
-- under the old session-scoped rule only the most recent registrant matched,
-- so every other tab's heartbeat silently updated zero rows and returned
-- success, and the shared row expired out from under tabs that were still
-- watching.
--
-- Refreshes the row-level `lastActiveAt` (the TTL sweep's authority) AND
-- re-stamps this session in `subscribers`, dropping its own previous entry and
-- everyone else's expired ones in the same atomic statement — so there is no
-- read-modify-write round trip and the array cannot grow across reconnects.
--
-- Idempotent and safe to lose: the client heartbeats at 0.9 * ttl, so an entry
-- is re-stamped well before `seenAt + ttl` would drop it.
--
-- No PERMISSIONS clause: a function body runs as the CALLER, so the table rule
-- is the real gate. (Contrast the fn::feature::* block, where the clause is
-- load-bearing.)
DEFINE FUNCTION OVERWRITE fn::query::heartbeat($id: record) {
    LET $sid = <string>session::id();
    LET $now = time::now();
    UPDATE $id SET
        lastActiveAt = $now,
        subscribers  = array::append(
            array::filter(subscribers ?? [], |$s|
                <string>$s.id != $sid AND <datetime>$s.seenAt + ttl > $now
            ),
            { id: $sid, seenAt: $now }
        );
};

-- Release THIS session's claim on a shared incantation.
--
-- Deletes the row — and therefore, via the `_00_dbsp_cleanup` event, the SSP
-- view and every `_00_list_ref` edge — ONLY when no other session is still
-- watching. Before this existed the client issued a bare `DELETE $id`, which
-- would have torn the view down for every other tab; in practice `_00_query`
-- granted no delete permission, so it silently did nothing at all and the TTL
-- sweep was the only reclamation that ever ran.
--
-- Stale entries are pruned on the way past, so a tab that crashed without
-- unsubscribing cannot keep a view alive forever. If this leaves the set
-- empty the row goes now; otherwise `lastActiveAt` and the sweep remain the
-- backstop.
--
-- Best-effort, not a guarantee: two tabs closing at the same instant can each
-- observe the other and neither delete, deferring release to TTL. That is
-- benign — the set is advisory — so do not build anything that assumes the
-- eager path is deterministic.
DEFINE FUNCTION OVERWRITE fn::query::unsubscribe($id: record) -> object {
    LET $sid = <string>session::id();
    LET $now = time::now();
    LET $row = (SELECT subscribers, ttl FROM ONLY $id);
    RETURN IF $row = NONE {
        { released: false, remaining: 0 }
    } ELSE {
        LET $rest = array::filter($row.subscribers ?? [], |$s|
            <string>$s.id != $sid AND <datetime>$s.seenAt + $row.ttl > $now
        );
        IF array::len($rest) = 0 {
            DELETE $id;
            { released: true, remaining: 0 }
        } ELSE {
            UPDATE $id SET subscribers = $rest;
            { released: false, remaining: array::len($rest) }
        }
    };
};
