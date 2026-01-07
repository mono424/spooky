import 'package:flutter_test/flutter_test.dart';
import 'package:flutter_core/core/services/sync/utils.dart'; // flattenRecords

void main() {
  test('flattenRecords flattens nested structures', () {
    // 1. Setup Relationship Map
    final relationships = {
      'thread': {'author'},
    };

    // 2. Setup Nested Data
    // Thread containing a full User object in 'author'
    final nestedData = [
      {
        'id': 'thread:1',
        'title': 'Test Thread',
        'author': {'id': 'user:1', 'username': 'Alice'},
      },
    ];

    // 3. Flatten
    final flattened = flattenRecords(nestedData, relationships);

    // 4. Verification
    // Should have 2 records: Thread and User
    expect(flattened.length, 2);

    // Verify User record exists
    final user = flattened.firstWhere((r) => r['id'] == 'user:1');
    expect(user, isNotNull);
    expect(user['username'], 'Alice');

    // Verify Thread record exists and author is normalized to ID
    final thread = flattened.firstWhere((r) => r['id'] == 'thread:1');
    expect(thread, isNotNull);
    expect(thread['title'], 'Test Thread');
    expect(thread['author'], 'user:1'); // Should be ID string now
  });

  test('flattenRecords handles list of nested records', () {
    final relationships = {
      'post': {'comments'},
    };

    final nestedData = [
      {
        'id': 'post:1',
        'comments': [
          {'id': 'comment:1', 'text': 'Nice'},
          {'id': 'comment:2', 'text': 'Cool'},
        ],
      },
    ];

    final flattened = flattenRecords(nestedData, relationships);

    expect(flattened.length, 3); // Post + 2 Comments

    final post = flattened.firstWhere((r) => r['id'] == 'post:1');
    expect(post['comments'], ['comment:1', 'comment:2']);

    final c1 = flattened.firstWhere((r) => r['id'] == 'comment:1');
    expect(c1['text'], 'Nice');
  });
}
