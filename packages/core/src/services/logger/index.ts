import pino, { type Logger as PinoLogger } from 'pino';
import { Level } from 'pino';

export type Logger = PinoLogger;

function getLevelLabel(levelVal: number) {
  if (levelVal >= 50) return 'ERROR';
  if (levelVal >= 40) return 'WARN';
  return 'INFO';
}

export function createLogger(level: Level = 'info'): Logger {
  return pino({
    level,
    browser: {
      asObject: true,
      write: (o) => {
        console.log(JSON.stringify(o));
      },
    },
    // We can add a custom serializer or transport if needed,
    // but default JSON is standard for pino.
  });
}
