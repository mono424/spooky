import 'dart:io';

import 'package:spooky_core/codegen.dart';
import 'package:test/test.dart';

const _schema = '''
DEFINE ACCESS account ON DATABASE TYPE RECORD
  SIGNUP ( CREATE user SET email = \$email, password = \$password )
  SIGNIN ( SELECT * FROM user WHERE email = \$email );
DEFINE TABLE user SCHEMAFULL PERMISSIONS FOR select WHERE true;
DEFINE FIELD email ON user TYPE string;
DEFINE TABLE thread SCHEMAFULL PERMISSIONS FOR select WHERE true;
DEFINE FIELD title ON thread TYPE string;
DEFINE FIELD author ON thread TYPE record<user>;
DEFINE FIELD score ON thread TYPE int;
DEFINE FIELD created_at ON thread TYPE option<datetime>;
''';

const _openapi = '''
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
                id: { type: string }
              required: [id]
''';

void main() {
  final backends = [parseOpenApi('api', _openapi)];

  group('typed emitter output', () {
    late String src;
    setUpAll(() => src = generateDartSource(_schema, backends: backends));

    test('field tokens with the right TypedField subtype per field', () {
      expect(src, contains('abstract final class Thread\$ {'));
      expect(src, contains("static const id = StringField('id');"));
      expect(src, contains("static const title = StringField('title');"));
      expect(src, contains("static const author = RecordField('author');"));
      expect(src, contains("static const score = NumField('score');"));
      expect(src,
          contains("static const created_at = DateTimeField('created_at');"));
    });

    test('all-nullable Patch with null-omitting toJson', () {
      expect(src, contains('class ThreadPatch {'));
      expect(src, contains('final String? title;'));
      expect(src, contains("if (title != null) m['title'] = title;"));
    });

    test('typed collection: query/create/update/delete', () {
      expect(src, contains('class ThreadCollection {'));
      expect(src, contains('TypedQuery<Thread> query() =>'));
      expect(src, contains('Future<void> create(Thread model) =>'));
      expect(src,
          contains('Future<void> update(String id, ThreadPatch patch) =>'));
      expect(src, contains('Future<void> delete(String id) =>'));
    });

    test('typed auth methods per access (String params)', () {
      expect(src, contains('class AuthApi {'));
      // SIGNIN references only $email; SIGNUP references $email + $password.
      expect(src, contains('signInAccount({required String email})'));
      expect(
          src,
          contains(
              'signUpAccount({required String email, required String password})'));
    });

    test('typed backends with required-arg routes', () {
      expect(src, contains('class Backends {'));
      expect(src, contains('ApiBackend get api =>'));
      expect(src, contains('class ApiBackend {'));
      expect(src, contains('Future<void> spookify({required String id}) =>'));
      expect(src, contains("_c.run('api', '/spookify', {'id': id});"));
    });

    test('AppDb facade wires tables + run + auth', () {
      expect(src, contains('class AppDb {'));
      expect(src, contains('factory AppDb.open(DatabaseConfig database)'));
      expect(src,
          contains('ThreadCollection get thread => ThreadCollection(client);'));
      expect(src, contains('Backends get run => Backends(client);'));
      expect(src, contains('AuthApi get auth => AuthApi(client);'));
    });
  });

  group('golden compile', () {
    test('generated typed client passes dart analyze', () async {
      final src = generateDartSource(_schema, backends: backends);
      final tmp = File('${Directory.current.path}/_typed_gen_check.dart');
      tmp.writeAsStringSync(src);
      addTearDown(() {
        if (tmp.existsSync()) tmp.deleteSync();
      });
      final result = await Process.run('dart', ['analyze', tmp.path]);
      expect(result.exitCode, 0,
          reason: 'generated client should analyze cleanly:\n'
              '${result.stdout}\n${result.stderr}');
    }, timeout: const Timeout(Duration(minutes: 2)));
  });
}
