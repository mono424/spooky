import 'dart:async';

import '../services/logger/logger.dart';

/// Retry [operation] with linear backoff on transient transaction/busy errors.
///
/// Faithful port of TS `withRetry`: retries only when the error message mentions
/// a transaction conflict or a busy database; backoff is `delayMs * (i + 1)`.
Future<T> withRetry<T>(
  SpookyLogger logger,
  Future<T> Function() operation, {
  int retries = 3,
  int delayMs = 100,
}) async {
  Object? lastError;
  for (var i = 0; i < retries; i++) {
    try {
      return await operation();
    } catch (err) {
      lastError = err;
      final msg = err.toString();
      if (msg.contains('Can not open transaction') ||
          msg.contains('transaction') ||
          msg.contains('Database is busy')) {
        logger.warn('Retrying DB operation (attempt ${i + 1}/$retries): $msg');
        await Future<void>.delayed(Duration(milliseconds: delayMs * (i + 1)));
        continue;
      }
      rethrow;
    }
  }
  throw lastError!;
}
