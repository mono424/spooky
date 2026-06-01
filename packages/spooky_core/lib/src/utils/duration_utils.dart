import '../surreal/value.dart';

/// Query time-to-live, e.g. `'10m'`, `'1h'`, `'1d'`. Mirrors the TS string
/// union `QueryTimeToLive`; kept as a plain string for fidelity and so raw
/// values round-trip unchanged.
typedef QueryTimeToLive = String;

/// Default TTL used across the client (TS default `'10m'`).
const QueryTimeToLive defaultTtl = '10m';

/// Parse a duration string (`<n>[smh]`) or [SurrealDuration] to milliseconds.
///
/// Faithful port of TS `parseDuration`: unrecognized input falls back to
/// 600000ms (10 minutes), and a bare number unit defaults to minutes.
int parseDuration(Object duration) {
  if (duration is SurrealDuration) {
    return parseDuration(duration.value);
  }
  if (duration is int) {
    return duration;
  }
  if (duration is! String) {
    return 600000;
  }

  final match = RegExp(r'^(\d+)([smh])$').firstMatch(duration);
  if (match == null) return 600000;
  final val = int.parse(match.group(1)!);
  final unit = match.group(2);
  switch (unit) {
    case 's':
      return val * 1000;
    case 'h':
      return val * 3600000;
    case 'm':
    default:
      return val * 60000;
  }
}
