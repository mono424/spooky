import { Layer, Logger, LogLevel } from "effect";
import type { LogLevel as LogLevelType } from "effect/LogLevel";
import type { SpookyConfig } from "./config.js";
import type { SchemaStructure } from "@spooky/query-builder";

/**
 * Map string log levels to Effect's LogLevel values
 */
const logLevelMap: Record<string, LogLevelType> = {
  debug: LogLevel.Debug,
  info: LogLevel.Info,
  warn: LogLevel.Warning,
  error: LogLevel.Error,
};

/**
 * Convert config log level string to Effect LogLevel
 */
export const toEffectLogLevel = (level: string): LogLevelType => {
  return logLevelMap[level.toLowerCase()] ?? LogLevel.Info;
};

/**
 * Create a formatted logger that displays log messages with timestamp and level
 */
const createFormattedLogger = (): Logger.Logger<unknown, void> =>
  Logger.make(({ logLevel, message, cause }) => {
    const timestamp = new Date().toISOString();
    const levelLabel = logLevel.label;
    const causeMessage = cause._tag === "Empty" ? "" : ` (${cause})`;
    const output = `[${timestamp}] [${levelLabel}] ${String(
      message
    )}${causeMessage}`;
    console.log(output);
  });

/**
 * Create a logger layer with a fixed log level
 * Use this with Effect.provide() or include in a Layer.mergeAll()
 */
export const makeLoggerLayer = (
  logLevel: "debug" | "info" | "warn" | "error"
) => {
  const effectLogLevel = toEffectLogLevel(logLevel);
  const formattedLogger = createFormattedLogger();

  const customLoggerLayer = Logger.replace(
    Logger.defaultLogger,
    formattedLogger
  );
  const minLogLevelLayer = Logger.minimumLogLevel(effectLogLevel);

  return Layer.mergeAll(customLoggerLayer, minLogLevelLayer);
};

/**
 * Create a logger layer from SpookyConfig
 * This reads the logLevel from the config and applies it
 *
 * @param config - The SpookyConfig with logLevel setting
 * @returns A Layer that can be used with Effect.provide()
 */
export const LoggerLayer = <S extends SchemaStructure>(
  config: SpookyConfig<S>
) => {
  const effectLogLevel = toEffectLogLevel(config.logLevel);
  const formattedLogger = createFormattedLogger();

  const customLoggerLayer = Logger.replace(
    Logger.defaultLogger,
    formattedLogger
  );
  const minLogLevelLayer = Logger.minimumLogLevel(effectLogLevel);

  return Layer.mergeAll(customLoggerLayer, minLogLevelLayer);
};
