import pino, { Level, type Logger as PinoLogger, type LoggerOptions } from 'pino';
import { LoggerProvider, BatchLogRecordProcessor } from '@opentelemetry/sdk-logs';
import { OTLPLogExporter } from '@opentelemetry/exporter-logs-otlp-proto';
import { resourceFromAttributes } from '@opentelemetry/resources';
import { ATTR_SERVICE_NAME } from '@opentelemetry/semantic-conventions';
import { createContextKey } from '@opentelemetry/api';

const CATEGORY_KEY = createContextKey('Category');

export type Logger = PinoLogger;

// Map pino levels to OTEL severity numbers
// https://github.com/open-telemetry/opentelemetry-specification/blob/main/specification/logs/data-model.md#severity-fields
function mapLevelToSeverityNumber(level: string): number {
  switch (level) {
    case 'trace':
      return 1;
    case 'debug':
      return 5;
    case 'info':
      return 9;
    case 'warn':
      return 13;
    case 'error':
      return 17;
    case 'fatal':
      return 21;
    default:
      return 9;
  }
}

export function createLogger(level: Level = 'info', otelEndpoint?: string): Logger {
  const browserConfig: LoggerOptions['browser'] = {
    asObject: true,
    write: (o: any) => {
      console.log(JSON.stringify(o));
    },
  };

  if (otelEndpoint) {
    // Initialize OTEL LoggerProvider
    const resource = resourceFromAttributes({
      [ATTR_SERVICE_NAME]: 'spooky-client',
    });

    const exporter = new OTLPLogExporter({
      url: otelEndpoint,
    });

    // Pass processors in constructor as this SDK version requires it
    const loggerProvider = new LoggerProvider({
      resource,
      processors: [new BatchLogRecordProcessor(exporter)],
    });

    const otelLogger: Record<string, ReturnType<typeof loggerProvider.getLogger>> = {};

    const getOtelLogger = (category: string) => {
      if (!otelLogger[category]) {
        otelLogger[category] = loggerProvider.getLogger(category);
      }
      return otelLogger[category];
    };

    browserConfig.transmit = {
      level: level,
      send: (levelLabel: string, logEvent: any) => {
        try {
          const messages = [...logEvent.messages];
          const severityNumber = mapLevelToSeverityNumber(levelLabel);

          // Construct the message body
          let body = '';
          const msg = messages.pop();

          if (typeof msg === 'string') {
            body = msg;
          } else if (msg) {
            body = JSON.stringify(msg);
          }

          let category = 'spooky-client::unknown';

          const attributes = {};
          for (const msg of messages) {
            if (typeof msg === 'object') {
              if (msg.Category) {
                category = msg.Category;
                delete msg.Category;
              }
              Object.assign(attributes, msg);
            }
          }

          // Emit to OTEL SDK
          getOtelLogger(category).emit({
            severityNumber: severityNumber,
            severityText: levelLabel.toUpperCase(),
            body: body,
            attributes: {
              ...logEvent.bindings[0],
              ...attributes,
            },
            timestamp: new Date(logEvent.ts),
          });
        } catch (e) {
          console.warn('Failed to transmit log to OTEL endpoint', e);
        }
      },
    };
  }

  return pino({
    level,
    browser: browserConfig,
  });
}
