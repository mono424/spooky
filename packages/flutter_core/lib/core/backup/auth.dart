import 'db.dart';

class AuthManager {
  final DatabaseService db;

  AuthManager(this.db);

  Future<void> authenticate(String token) async {
    await db.getRemote().authenticate(token: token);
  }

  Future<void> deauthenticate() async {
    await db.getRemote().invalidate();
  }
}
