export type EventDefinition<T extends string, P> = {
  type: T;
} & ([P] extends [never] ? {} : { payload: P });

export type EventTypeMap = Record<
  string,
  EventDefinition<any, unknown> | EventDefinition<any, never>
>;

export type Event<E extends EventTypeMap, T extends EventType<E>> = E[T];

export type EventTypes<E extends EventTypeMap> = (keyof E)[];

export type EventType<E extends EventTypeMap> = keyof E;

export type EventHandler<E extends EventTypeMap, T extends EventType<E>> = (
  event: Event<E, T>
) => void;

type InnerEventHandler<E extends EventTypeMap, T extends EventType<E>> = {
  id: number;
  handler: EventHandler<E, T>;
  once?: boolean;
};

export type EventSubscriptionOptions = {
  immediately?: boolean;
  once?: boolean;
};

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

  get eventTypes(): EventTypes<E> {
    return this.eventTypes;
  }

  constructor(private _eventTypes: EventTypes<E>) {
    this.buffer = [];
    this.subscribers = this._eventTypes.reduce((acc, key) => {
      return Object.assign(acc, { [key]: new Map() });
    }, {} as { [K in EventType<E>]: Map<number, InnerEventHandler<E, K>> });
    this.lastEvents = {};
    this.subscribersTypeMap = new Map();
  }

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

  unsubscribe(id: number): boolean {
    const type = this.subscribersTypeMap.get(id);
    if (type) {
      this.subscribers[type].delete(id);
      this.subscribersTypeMap.delete(id);
      return true;
    }
    return false;
  }

  addEvent<T extends EventType<E>>(event: Event<E, T>): void {
    this.buffer.push(event);
    this.scheduleProcessing();
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

  private setLastEvent<T extends EventType<E>>(
    type: T,
    event: Event<E, T>
  ): void {
    this.lastEvents[type] = event;
  }

  private broadcastEvent<T extends EventType<E>>(
    type: T,
    event: Event<E, T>
  ): void {
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
