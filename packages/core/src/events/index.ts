/**
 * Utility type to define the payload structure of an event.
 * If the payload type P is never, it defines payload as undefined.
 */
export type EventPayloadDefinition<P> = [P] extends [never]
  ? { payload: undefined }
  : { payload: P };

/**
 * Defines the structure of an event with a specific type and payload.
 * @template T The string literal type of the event.
 * @template P The type of the event payload.
 */
export type EventDefinition<T extends string, P> = {
  type: T;
} & EventPayloadDefinition<P>;

/**
 * A map of event types to their definitions.
 * Keys are event names, values are EventDefinitions.
 */
export type EventTypeMap = Record<
  string,
  EventDefinition<any, unknown> | EventDefinition<any, never>
>;

/**
 * Options for pushing/emitting events.
 */
export interface PushEventOptions {
  /** Configuration for debouncing the event. */
  debounced?: { key: string; delay: number };
}

/**
 * Extracts the full Event object type from the map for a given key.
 */
export type Event<E extends EventTypeMap, T extends EventType<E>> = E[T];

/**
 * Extracts the payload type from the map for a given key.
 */
export type EventPayload<E extends EventTypeMap, T extends EventType<E>> = E[T]['payload'];

/**
 * Array of available event type keys.
 */
export type EventTypes<E extends EventTypeMap> = (keyof E)[];

/**
 * Represents a valid key (event name) from the EventTypeMap.
 */
export type EventType<E extends EventTypeMap> = keyof E;

/**
 * Function signature for an event handler.
 */
export type EventHandler<E extends EventTypeMap, T extends EventType<E>> = (
  event: Event<E, T>
) => void;

type InnerEventHandler<E extends EventTypeMap, T extends EventType<E>> = {
  id: number;
  handler: EventHandler<E, T>;
  once?: boolean;
};

/**
 * Options when subscribing to an event.
 */
export type EventSubscriptionOptions = {
  /** If true, the handler will be called immediately with the last emitted event of this type (if any). */
  immediately?: boolean;
  /** If true, the subscription will be automatically removed after the first event is handled. */
  once?: boolean;
};

/**
 * A type-safe event system that handles subscription, emission (including debouncing), and buffering of events.
 * @template E The EventTypeMap defining all supported events.
 */
export class EventSystem<E extends EventTypeMap> {
  private subscriberId: number = 0;
  private isProcessing: boolean = false;
  private buffer: Event<E, EventType<E>>[];
  private subscribers: {
    [K in EventType<E>]: Map<number, InnerEventHandler<E, K>>;
  };
  private subscribersTypeMap: Map<number, EventType<E>>;
  private lastEvents: {
    [K in EventType<E>]?: Event<E, K>;
  };

  private debouncedEvents: Map<string, { timer: any; resolve: (val: any) => void }>;

  constructor(private _eventTypes: EventTypes<E>) {
    this.buffer = [];
    this.subscribers = this._eventTypes.reduce(
      (acc, key) => {
        return Object.assign(acc, { [key]: new Map() });
      },
      {} as { [K in EventType<E>]: Map<number, InnerEventHandler<E, K>> }
    );
    this.lastEvents = {};
    this.subscribersTypeMap = new Map();
    this.debouncedEvents = new Map();
  }

  get eventTypes(): EventTypes<E> {
    return this._eventTypes;
  }

  /**
   * Subscribes a handler to a specific event type.
   * @param type The event type to subscribe to.
   * @param handler The function to call when the event occurs.
   * @param options Subscription options (once, immediately).
   * @returns A subscription ID that can be used to unsubscribe.
   */
  subscribe<T extends EventType<E>>(
    type: T,
    handler: EventHandler<E, T>,
    options?: EventSubscriptionOptions
  ): number {
    const id = this.subscriberId++;
    this.subscribers[type].set(id, {
      id,
      handler,
      once: options?.once ?? false,
    });
    this.subscribersTypeMap.set(id, type);
    if (options?.immediately) {
      const lastEvent = this.lastEvents[type];
      if (lastEvent) handler(lastEvent);
    }
    return id;
  }

  /**
   * Subscribes a handler to multiple event types.
   * @param types An array of event types to subscribe to.
   * @param handler The function to call when any of the events occur.
   * @param options Subscription options.
   * @returns An array of subscription IDs.
   */
  subscribeMany<T extends EventType<E>>(
    types: T[],
    handler: EventHandler<E, T>,
    options?: EventSubscriptionOptions
  ): number[] {
    return types.map((type) => this.subscribe(type, handler, options));
  }

  /**
   * Unsubscribes a specific subscription by ID.
   * @param id The subscription ID returned by subscribe().
   * @returns True if the subscription was found and removed, false otherwise.
   */
  unsubscribe(id: number): boolean {
    const type = this.subscribersTypeMap.get(id);
    if (type) {
      this.subscribers[type].delete(id);
      this.subscribersTypeMap.delete(id);
      return true;
    }
    return false;
  }

  /**
   * Emits an event with the given type and payload.
   * @param type The type of event to emit.
   * @param payload The data associated with the event.
   */
  emit<T extends EventType<E>, P extends EventPayload<E, T>>(type: T, payload: P): void {
    const event = {
      type,
      payload,
    } as unknown as Event<E, T>;
    this.addEvent(event);
  }

  /**
   * Adds a fully constructed event object to the system.
   * Similar to emit, but takes the full event object directly.
   * Supports debouncing if options are provided.
   * @param event The event object.
   * @param options Options for the event push (e.g., debouncing).
   */
  addEvent<T extends EventType<E>>(event: Event<E, T>, options?: PushEventOptions): void {
    if (options?.debounced) {
      this.handleDebouncedEvent(event, options.debounced.key, options.debounced.delay);
      return;
    }
    this.buffer.push(event);
    this.scheduleProcessing();
  }

  private handleDebouncedEvent<T extends EventType<E>>(
    event: Event<E, T>,
    key: string,
    delay: number
  ): void {
    if (this.debouncedEvents.has(key)) {
      clearTimeout(this.debouncedEvents.get(key)?.timer);
    }

    const timer = setTimeout(() => {
      this.debouncedEvents.delete(key);
      this.buffer.push(event);
      this.scheduleProcessing();
    }, delay);

    this.debouncedEvents.set(key, { timer, resolve: () => {} });
  }

  private scheduleProcessing(): void {
    if (!this.isProcessing) {
      queueMicrotask(() => this.processEvents());
    }
  }

  private async processEvents(): Promise<void> {
    if (this.isProcessing) return;
    this.isProcessing = true;

    try {
      while (this.dequeue());
    } finally {
      this.isProcessing = false;
    }
  }

  private dequeue(): boolean {
    const event = this.buffer.shift();
    if (!event) return false;

    this.setLastEvent(event.type, event);
    this.broadcastEvent(event.type, event);
    return true;
  }

  private setLastEvent<T extends EventType<E>>(type: T, event: Event<E, T>): void {
    this.lastEvents[type] = event;
  }

  private broadcastEvent<T extends EventType<E>>(type: T, event: Event<E, T>): void {
    const subscribers = this.subscribers[type].values();
    for (const subscriber of subscribers) {
      subscriber.handler(event);
      if (subscriber.once) {
        this.unsubscribe(subscriber.id);
      }
    }
  }
}

export function createEventSystem<E extends EventTypeMap>(
  eventTypes: EventTypes<E>
): EventSystem<E> {
  return new EventSystem(eventTypes);
}
