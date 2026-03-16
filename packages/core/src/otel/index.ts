import { Level } from 'pino';
import { PinoTransmit } from '../types';

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

async function loadOtelModules(otelEndpoint: string) {
  const [{ LoggerProvider, BatchLogRecordProcessor }, { OTLPLogExporter }, { resourceFromAttributes }, { ATTR_SERVICE_NAME }] =
    await Promise.all([
      import('@opentelemetry/sdk-logs'),
      import('@opentelemetry/exporter-logs-otlp-proto'),
      import('@opentelemetry/resources'),
      import('@opentelemetry/semantic-conventions'),
    ]);

  const resource = resourceFromAttributes({
    [ATTR_SERVICE_NAME]: 'spooky-client',
  });

  const exporter = new OTLPLogExporter({
    url: otelEndpoint,
  });

  const loggerProvider = new LoggerProvider({
    resource,
    processors: [new BatchLogRecordProcessor(exporter)],
  });

  const otelLoggerCache: Record<string, ReturnType<typeof loggerProvider.getLogger>> = {};

  return (category: string) => {
    if (!otelLoggerCache[category]) {
      otelLoggerCache[category] = loggerProvider.getLogger(category);
    }
    return otelLoggerCache[category];
  };
}

/**
 * Creates a pino browser transmit object that forwards logs to an OpenTelemetry collector.
 *
 * @example
 * ```ts
 * import { createOtelTransmit } from '@spooky-sync/core/otel';
 *
 * new SpookyClient({
 *   // ...
 *   otelTransmit: createOtelTransmit('http://localhost:4318/v1/logs'),
 * });
 * ```
 */
export function createOtelTransmit(endpoint: string, level: Level = 'info'): PinoTransmit {
  // Start loading OTel modules eagerly (don't await — we're synchronous)
  const otelReady = loadOtelModules(endpoint);

  return {
    level: level,
    send: (levelLabel: string, logEvent: any) => {
      otelReady.then((getOtelLogger) => {
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
      }).catch((e) => {
        console.warn('Failed to load OpenTelemetry modules', e);
      });
    },
  };
}
