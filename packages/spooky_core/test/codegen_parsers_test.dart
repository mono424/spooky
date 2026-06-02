import 'package:spooky_core/codegen.dart';
import 'package:test/test.dart';

void main() {
  group('parseAccesses', () {
    test('extracts signup/signin vars from DEFINE ACCESS (built-ins excluded)',
        () {
      const surql = '''
        DEFINE ACCESS account ON DATABASE TYPE RECORD
          SIGNUP {
            RETURN CREATE ONLY user SET
              username = \$username,
              password = crypto::argon2::generate(\$password),
              share_pubkey = \$share_pubkey;
          }
          SIGNIN ( SELECT * FROM user WHERE username = \$username
                   AND crypto::argon2::compare(password, \$password) )
          DURATION FOR TOKEN 15m, FOR SESSION 30d;
        DEFINE TABLE user SCHEMAFULL;
      ''';
      final accesses = parseAccesses(surql);
      expect(accesses, hasLength(1));
      final a = accesses.single;
      expect(a.name, 'account');
      expect(a.signupParams, ['username', 'password', 'share_pubkey']);
      expect(a.signinParams, ['username', 'password']);
    });

    test('parseProject returns tables + accesses together', () {
      const surql = '''
        DEFINE ACCESS account ON DATABASE TYPE RECORD
          SIGNUP ( CREATE user SET email = \$email, password = \$password )
          SIGNIN ( SELECT * FROM user WHERE email = \$email );
        DEFINE TABLE user SCHEMAFULL;
        DEFINE FIELD email ON user TYPE string;
      ''';
      final parsed = parseProject(surql);
      expect(parsed.tables.map((t) => t.name), contains('user'));
      expect(parsed.accesses.single.signupParams, ['email', 'password']);
      expect(parsed.accesses.single.signinParams, ['email']);
    });
  });

  group('parseOpenApi', () {
    const spec = '''
openapi: 3.1.0
paths:
  /spookify:
    post:
      requestBody:
        content:
          application/json:
            schema:
              type: object
              properties:
                id:
                  type: string
                count:
                  type: integer
              required:
                - id
  /noargs:
    post:
      responses:
        '200': { description: ok }
''';

    test('parses routes + typed args with required/optional', () {
      final backend = parseOpenApi('api', spec, outboxTable: 'job');
      expect(backend.name, 'api');
      expect(backend.outboxTable, 'job');
      expect(backend.routes.map((r) => r.path), ['/noargs', '/spookify']);

      final spookify = backend.routes.firstWhere((r) => r.path == '/spookify');
      final id = spookify.args.firstWhere((a) => a.name == 'id');
      final count = spookify.args.firstWhere((a) => a.name == 'count');
      expect(id.type, 'String');
      expect(id.optional, isFalse);
      expect(count.type, 'int');
      expect(count.optional, isTrue);

      final noargs = backend.routes.firstWhere((r) => r.path == '/noargs');
      expect(noargs.args, isEmpty);
    });
  });
}
