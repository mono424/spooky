import '../../events/event_system.dart';
import '../../services/database/remote_database_service.dart';
import '../../services/logger/logger.dart';
import '../../types.dart';
import '../../utils/parser.dart' show ColumnSchema;

/// Auth event type name (TS `AuthEventTypes`).
abstract final class AuthEventTypes {
  static const authStateChanged = 'AUTH_STATE_CHANGED';
}

EventSystem createAuthEventSystem() =>
    EventSystem([AuthEventTypes.authStateChanged]);

/// Auth state management (TS `AuthService`). The TS conditional access-param
/// types collapse to runtime `Map` validation against `schema['access']`.
class AuthService {
  AuthService(
      this._schema, this._remote, this._persistence, SpookyLogger logger)
      : _logger = logger.child('AuthService');

  final Map<String, dynamic> _schema;
  final RemoteDatabaseService _remote;
  final PersistenceClient _persistence;
  final SpookyLogger _logger;
  final EventSystem _events = createAuthEventSystem();

  static const _tokenKey = 'sp00ky_auth_token';

  String? token;
  Map<String, dynamic>? currentUser;
  bool isAuthenticated = false;
  bool isLoading = true;

  EventSystem get eventSystem => _events;

  Future<void> init() => check();

  Map<String, dynamic>? getAccessDefinition(String name) {
    final access = _schema['access'];
    if (access is Map) return (access[name] as Map?)?.cast<String, dynamic>();
    return null;
  }

  /// Subscribe to auth state. Fires immediately with the current user id.
  void Function() subscribe(void Function(String? userId) cb) {
    cb(currentUser?['id']?.toString());
    final id = _events.subscribe(
        AuthEventTypes.authStateChanged, (e) => cb(e.payload as String?));
    return () => _events.unsubscribe(id);
  }

  void _notifyListeners() {
    _events.emit(
        AuthEventTypes.authStateChanged, currentUser?['id']?.toString());
  }

  /// Validate an existing or supplied token and hydrate the user.
  Future<void> check([String? accessToken]) async {
    isLoading = true;
    try {
      final tok = accessToken ?? await _persistence.get<String>(_tokenKey);
      if (tok == null) {
        isLoading = false;
        isAuthenticated = false;
        _notifyListeners();
        return;
      }

      await _remote.getClient().authenticate(tok);
      final user = await _fetchAuthUser();
      if (user != null && user['id'] != null) {
        await _setSession(tok, user);
      } else {
        await signOut();
      }
    } catch (error) {
      _logger.error('Auth check failed', error);
      await signOut();
    } finally {
      isLoading = false;
    }
  }

  Future<Map<String, dynamic>?> _fetchAuthUser() async {
    final result = await _remote.query('SELECT * FROM ONLY \$auth.id');
    final first = result.isNotEmpty ? result.first : null;
    if (first is List)
      return first.isNotEmpty
          ? (first.first as Map?)?.cast<String, dynamic>()
          : null;
    if (first is Map) return first.cast<String, dynamic>();
    return null;
  }

  Future<void> signOut() async {
    token = null;
    currentUser = null;
    isAuthenticated = false;
    await _persistence.remove(_tokenKey);
    try {
      await _remote.getClient().invalidate();
    } catch (_) {
      // ignore invalidation errors
    }
    _notifyListeners();
  }

  Future<void> _setSession(String token, Map<String, dynamic> user) async {
    this.token = token;
    currentUser = user;
    isAuthenticated = true;
    await _persistence.set(_tokenKey, token);
    _notifyListeners();
  }

  Future<void> signUp(String accessName, Map<String, dynamic> params) async {
    _validateAccessParams(accessName, 'signup', params);
    final result =
        await _remote.signup({'access': accessName, 'variables': params});
    await check(_extractAccessToken(result));
  }

  Future<void> signIn(String accessName, Map<String, dynamic> params) async {
    _validateAccessParams(accessName, 'signIn', params);
    final result =
        await _remote.signin({'access': accessName, 'variables': params});
    await check(_extractAccessToken(result));
  }

  void _validateAccessParams(
      String accessName, String method, Map<String, dynamic> params) {
    final def = getAccessDefinition(accessName);
    if (def == null) {
      throw StateError("Access definition '$accessName' not found");
    }
    final methodDef = (def[method] as Map?)?.cast<String, dynamic>();
    final declared =
        (methodDef?['params'] as Map?)?.cast<String, dynamic>() ?? {};
    final missing = <String>[];
    declared.forEach((name, schema) {
      final optional = schema is ColumnSchema
          ? schema.optional
          : (schema is Map && schema['optional'] == true);
      if (!optional && !params.containsKey(name)) missing.add(name);
    });
    if (missing.isNotEmpty) {
      throw StateError(
          "Missing required $method params for '$accessName': ${missing.join(', ')}");
    }
  }

  String? _extractAccessToken(dynamic result) {
    if (result is Map && result['access'] != null)
      return result['access'].toString();
    if (result is String) return result;
    return null;
  }
}
