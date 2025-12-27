import { LogLevel } from '../types.js';

export interface Logger {
  debug(message: string, ...args: any[]): void;
  info(message: string, ...args: any[]): void;
  warn(message: string, ...args: any[]): void;
  error(message: string, ...args: any[]): void;
}

export function createLogger(logLevel: LogLevel = 'info'): Logger {
  const levels: Record<LogLevel, number> = {
    debug: 0,
    info: 1,
    warn: 2,
    error: 3,
  };

  const currentLevel = levels[logLevel];

  const formatMessage = (level: string, message: string, ...args: any[]): void => {
    const timestamp = new Date().toISOString();
    const output = `[${timestamp}] [${level}] ${message}`;
    if (args.length > 0) {
      console.log(output, ...args);
    } else {
      console.log(output);
    }
  };

  return {
    debug: (message: string, ...args: any[]) => {
      if (currentLevel <= levels.debug) {
        formatMessage('DEBUG', message, ...args);
      }
    },
    info: (message: string, ...args: any[]) => {
      if (currentLevel <= levels.info) {
        formatMessage('INFO', message, ...args);
      }
    },
    warn: (message: string, ...args: any[]) => {
      if (currentLevel <= levels.warn) {
        formatMessage('WARN', message, ...args);
      }
    },
    error: (message: string, ...args: any[]) => {
      if (currentLevel <= levels.error) {
        formatMessage('ERROR', message, ...args);
      }
    },
  };
}
