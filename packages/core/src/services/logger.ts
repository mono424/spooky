import pino, { type Logger as PinoLogger } from 'pino';
import { Level } from 'pino';

export type Logger = PinoLogger;

export function createLogger(level: Level = 'info'): Logger {
  return pino({
    level,
    browser: {
      asObject: true,
    },
    // We can add a custom serializer or transport if needed,
    // but default JSON is standard for pino.
  });
}
