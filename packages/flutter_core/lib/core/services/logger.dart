class Logger {
  void info(String message, [Map<String, dynamic>? meta]) {
    print('INFO: $message ${meta ?? ''}');
  }

  void debug(String message, [Map<String, dynamic>? meta]) {
    print('DEBUG: $message ${meta ?? ''}');
  }

  void error(String message, [Object? error]) {
    print('ERROR: $message ${error ?? ''}');
  }
}
