import { describe, it, expect } from 'vitest';
import { classifySyncError } from './error-classification';

describe('classifySyncError', () => {
  it('classifies surreal ConnectionUnavailableError as network', () => {
    // Thrown synchronously by the WS engine's send() while the socket is down
    // but reconnect hasn't fired. Message contains "connected", not "connection".
    const err = new Error(
      'You must be connected to a SurrealDB instance before performing this operation'
    );
    expect(classifySyncError(err)).toBe('network');
  });

  it('classifies surreal CallTerminatedError as network', () => {
    const err = new Error(
      'The call has been terminated because the connection was closed'
    );
    expect(classifySyncError(err)).toBe('network');
  });

  it.each([
    'WebSocket connection failed',
    'fetch failed',
    'connect ECONNREFUSED 127.0.0.1:8666',
    'request timed out',
    'The operation was aborted',
    'socket hang up',
  ])('classifies %j as network', (message) => {
    expect(classifySyncError(new Error(message))).toBe('network');
  });

  it.each([
    'Permission denied',
    'There was a problem with the database: record already exists',
    'Parse error: unexpected token',
  ])('classifies %j as application', (message) => {
    expect(classifySyncError(new Error(message))).toBe('application');
  });

  it('handles non-Error values', () => {
    expect(classifySyncError('connection reset')).toBe('network');
    expect(classifySyncError('boom')).toBe('application');
  });
});
