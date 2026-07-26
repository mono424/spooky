import 'package:spooky_core/spooky_core.dart';
import 'package:test/test.dart';

void main() {
  group('semverGt', () {
    test('compares major/minor/patch', () {
      expect(semverGt('1.0.1', '1.0.0'), isTrue);
      expect(semverGt('1.1.0', '1.0.9'), isTrue);
      expect(semverGt('2.0.0', '1.9.9'), isTrue);
      expect(semverGt('1.0.0', '1.0.1'), isFalse);
      expect(semverGt('1.0.0', '1.0.0'), isFalse);
    });

    test('reads missing parts as zero', () {
      expect(semverGt('1.2', '1.2.0'), isFalse);
      expect(semverGt('1.2.1', '1.2'), isTrue);
      expect(semverGt('2', '1.9.9'), isTrue);
    });

    test('malformed input never compares greater', () {
      expect(semverGt('abc', '1.0.0'), isFalse);
      expect(semverGt('9.9.9', 'abc'), isFalse);
      expect(semverGt('1.0.0-beta', '1.0.0'), isFalse);
      expect(semverGt('1.2.3.4', '1.0.0'), isFalse);
      expect(semverGt('', '1.0.0'), isFalse);
      expect(semverGt(null, '1.0.0'), isFalse);
      expect(semverGt('1.0.0', null), isFalse);
    });

    test('tolerates surrounding whitespace', () {
      expect(semverGt(' 1.0.1 ', '1.0.0'), isTrue);
    });
  });
}
