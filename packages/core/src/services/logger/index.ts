import pino, { Level, type Logger as PinoLogger, type LoggerOptions } from 'pino';
import { PinoTransmit } from '../../types';

export type Logger = PinoLogger;

export function createLogger(level: Level = 'info', transmit?: PinoTransmit): Logger {
  const browserConfig: LoggerOptions['browser'] = {
    asObject: true,
    write: (o: any) => {
      console.log(JSON.stringify(o));
    },
  };

  if (transmit) {
    browserConfig.transmit = transmit;
  }

  return pino({
    level,
    browser: browserConfig,
  });
}
