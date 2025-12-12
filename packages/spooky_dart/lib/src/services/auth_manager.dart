import 'database_service.dart';
import 'event_system.dart';

class AuthManager {
  final DatabaseService _db;
  final EventSystem _events;

  String? _currentUser;
  String? get currentUser => _currentUser;

  AuthManager(this._db, this._events);

  Future<void> signin(String username, String password) async {
    // In SurrealDB, signin typically involves a query or a specific method.
    // Since we are using the embedded engine, we might just be setting the user context
    // or running a SIGNIN query.
    // The engine `connect_db` already sets namespace/db.
    // Let's assume we run a SIGNIN query.

    final query =
        "SIGNIN (SELECT * FROM user WHERE username = '$username' AND crypto::argon2::compare(password, '$password'))";

    try {
      final res = await _db.query(query);
      // If successful, we get a token or user record.
      // Assuming the first result is the token/user.
      if (res.isNotEmpty && res[0] != null) {
        _currentUser = username;
        _events.emit(SpookyEventType.authChanged, {
          'user': username,
          'status': 'signedIn',
        });
      } else {
        throw Exception("Invalid credentials");
      }
    } catch (e) {
      _events.emit(SpookyEventType.error, "Signin failed: $e");
      rethrow;
    }
  }

  Future<void> signup(String username, String password) async {
    final query =
        "SIGNUP (CREATE user SET username = '$username', password = crypto::argon2::generate('$password'))";

    try {
      final res = await _db.query(query);
      if (res.isNotEmpty && res[0] != null) {
        _currentUser = username;
        _events.emit(SpookyEventType.authChanged, {
          'user': username,
          'status': 'signedUp',
        });
      } else {
        throw Exception("Signup failed");
      }
    } catch (e) {
      _events.emit(SpookyEventType.error, "Signup failed: $e");
      rethrow;
    }
  }

  Future<void> signout() async {
    _currentUser = null;
    // Invalidate session in DB if needed
    _events.emit(SpookyEventType.authChanged, {'status': 'signedOut'});
  }
}
