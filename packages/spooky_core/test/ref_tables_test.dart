import 'package:spooky_core/src/modules/ref_tables.dart';
import 'package:test/test.dart';

void main() {
  group('listRefTableFor', () {
    test('regular user → per-user table in dedicated mode', () {
      expect(listRefTableFor(RefMode.dedicated, 'user:abc'),
          '_00_list_ref_user_abc');
      expect(listRefTableFor(RefMode.dedicated, 'abc'),
          '_00_list_ref_user_abc');
    });

    test('single mode → the global table', () {
      expect(listRefTableFor(RefMode.single, 'user:abc'), '_00_list_ref');
    });

    test('null / unsanitizable user → the global table', () {
      expect(listRefTableFor(RefMode.dedicated, null), '_00_list_ref');
      expect(listRefTableFor(RefMode.dedicated, 'user:bad id!'),
          '_00_list_ref');
    });

    group('anonymous sentinel', () {
      test('resolves to the shared _00_list_ref_anon in both modes', () {
        expect(listRefTableFor(RefMode.dedicated, anonUserId),
            '_00_list_ref_anon');
        expect(listRefTableFor(RefMode.single, anonUserId),
            '_00_list_ref_anon');
      });

      test('a real user record "user:anon" is NOT the sentinel', () {
        // The sentinel carries no `user:` prefix, so it can never collide with
        // a real user id — `user:anon` must route to its own per-user table.
        expect(listRefTableFor(RefMode.dedicated, 'user:anon'),
            '_00_list_ref_user_anon');
      });
    });
  });
}
