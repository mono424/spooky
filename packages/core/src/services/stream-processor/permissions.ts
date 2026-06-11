/**
 * Extract per-table `select` permission predicates from a raw `.surql` schema.
 *
 * The in-browser SSP default-denies any non-`_00_` table that has no permission
 * predicate registered (see `permission_inject::build_predicate`). The client
 * already holds the full schema text (`config.schemaSurql`), so we parse each
 * `DEFINE TABLE … PERMISSIONS …` clause and hand the `select` predicate to the
 * processor via `set_permissions` at boot — mirroring the native path that
 * seeds the same map from `INFO FOR DB`.
 *
 * Returns `{ [table]: whereText }` where `whereText` is the raw SurrealQL
 * expression (`'true'` for `FULL`, `'false'` for `NONE` or a table with no
 * `select` permission).
 */
export function extractSelectPermissions(schemaSurql: string): Record<string, string> {
  const out: Record<string, string> = {};
  if (!schemaSurql) return out;

  // Drop line comments so a `--` comment can't contain a stray PERMISSIONS/FOR.
  const cleaned = schemaSurql.replace(/--[^\n]*/g, '');

  // One entry per `DEFINE TABLE … ;` statement.
  const tableStmt = /DEFINE\s+TABLE\s+(?:OVERWRITE\s+|IF\s+NOT\s+EXISTS\s+)?([A-Za-z_][\w]*)\b([^;]*);/gi;
  let m: RegExpExecArray | null;
  while ((m = tableStmt.exec(cleaned)) !== null) {
    const table = m[1];
    const body = m[2];
    out[table] = selectPredicateFromBody(body);
  }
  return out;
}

function selectPredicateFromBody(body: string): string {
  const permIdx = body.search(/\bPERMISSIONS\b/i);
  if (permIdx === -1) return 'false'; // no clause → SurrealDB default-deny
  const perms = body.slice(permIdx + 'PERMISSIONS'.length).trim();

  if (/^FULL\b/i.test(perms)) return 'true';
  if (/^NONE\b/i.test(perms)) return 'false';

  // `FOR <actions> WHERE <expr>` groups, possibly several. Split on the FOR
  // boundary; the leading empty segment (before the first FOR) is ignored.
  const groups = perms.split(/\bFOR\b/i).map((g) => g.trim()).filter(Boolean);
  for (const group of groups) {
    const where = group.search(/\bWHERE\b/i);
    if (where === -1) continue;
    const actions = group.slice(0, where).toLowerCase();
    if (/\bselect\b/.test(actions)) {
      return group.slice(where + 'WHERE'.length).trim();
    }
  }
  return 'false'; // permissions present but none grant select
}
