/// Client-side mirror of `ssp-protocol`'s `list_ref_table_for`. Same naming so
/// the LIVE subscription, the initial fetch, and the SSP writes land on the
/// same table. Faithful port of `modules/ref-tables.ts`.

enum RefMode { single, dedicated }

/// Default ref-storage mode (mirrors the SSP default `RefMode::Dedicated`).
const RefMode defaultRefMode = RefMode.dedicated;

/// Sentinel user id for unauthenticated clients when anonymous live queries are
/// enabled. Mirrors `ssp_protocol::ANON_AUTH_ID` and the TS `ANON_USER_ID`. It
/// carries no `user:` prefix so it can never collide with a real user id (those
/// arrive as `user:<id>`); both sides resolve it to the shared
/// `_00_list_ref_anon` table.
const String anonUserId = 'anon';

final _userIdPattern = RegExp(r'^[A-Za-z0-9_]+$');

/// Sanitize a user id (`"user:abc"` -> `"abc"`). Returns null when the id is
/// empty or contains characters invalid in a SurrealDB table identifier.
String? sanitizeUserId(Object? userId) {
  if (userId == null) return null;
  final asString = userId.toString();
  if (asString.isEmpty) return null;
  final raw = asString.startsWith('user:')
      ? asString.substring('user:'.length)
      : asString;
  if (raw.isEmpty) return null;
  if (!_userIdPattern.hasMatch(raw)) return null;
  return raw;
}

/// `_00_list_ref` table name for `(mode, userId)`. Falls back to the global
/// table in single mode or when sanitization fails.
String listRefTableFor(RefMode mode, Object? userId) {
  // Anonymous clients (flag-enabled) share one dedicated table in both modes —
  // checked before the mode split so it never lands on the per-user or the
  // auth-gated global table. Matches `ssp_protocol::list_ref_table_for`.
  if (userId == anonUserId) return '_00_list_ref_anon';
  if (mode == RefMode.single) return '_00_list_ref';
  final uid = sanitizeUserId(userId);
  return uid != null ? '_00_list_ref_user_$uid' : '_00_list_ref';
}
