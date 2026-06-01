import 'package:logging/logging.dart';

/// Lightweight logger facade over `package:logging`, standing in for the JS
/// `pino` logger. Supports the `logger.child({name})` pattern the modules use.
class SpookyLogger {
  SpookyLogger(this._logger);

  factory SpookyLogger.root([String name = 'spooky']) =>
      SpookyLogger(Logger(name));

  final Logger _logger;

  /// Create a named child logger (TS `logger.child({ name })`).
  SpookyLogger child(String name) =>
      SpookyLogger(Logger('${_logger.name}.$name'));

  void trace(Object? message) => _logger.finest(message);
  void debug(Object? message) => _logger.fine(message);
  void info(Object? message) => _logger.info(message);
  void warn(Object? message) => _logger.warning(message);
  void error(Object? message, [Object? err, StackTrace? stack]) =>
      _logger.severe(message, err, stack);
}
