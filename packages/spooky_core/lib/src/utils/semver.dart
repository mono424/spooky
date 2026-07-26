/// Minimal semver comparison for app-release version checks (TS
/// `utils/semver.ts`). `X.Y.Z` numeric with missing parts read as 0
/// (`1.2` == `1.2.0`); any malformed input never compares greater, so a bad
/// release row can never nag (or force-reload) every client.

final _digits = RegExp(r'^\d+$');

List<int>? _parse(Object? v) {
  final parts = (v?.toString() ?? '').trim().split('.');
  if (parts.isEmpty || parts.length > 3 || parts[0] == '') return null;
  final nums = <int>[];
  for (var i = 0; i < 3; i++) {
    final raw = i < parts.length ? parts[i] : '0';
    if (!_digits.hasMatch(raw)) return null;
    nums.add(int.parse(raw));
  }
  return nums;
}

/// True when [a] is a valid version strictly greater than valid version [b].
bool semverGt(Object? a, Object? b) {
  final pa = _parse(a);
  final pb = _parse(b);
  if (pa == null || pb == null) return false;
  for (var i = 0; i < 3; i++) {
    if (pa[i] > pb[i]) return true;
    if (pa[i] < pb[i]) return false;
  }
  return false;
}
