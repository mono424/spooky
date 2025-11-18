import { RecordId } from "@spooky/query-builder";

export type EventDefinition<T extends EventType, P> = {
  type: T;
  payload: P;
};

export const AuthEventTypes = {
  Authenticated: "AUTHENTICATED",
  Deauthenticated: "DEAUTHENTICATED",
} as const;

export type EventTypeMap = {
  [AuthEventTypes.Authenticated]: EventDefinition<
    typeof AuthEventTypes.Authenticated,
    {
      userId: RecordId;
      token: string;
    }
  >;
  [AuthEventTypes.Deauthenticated]: EventDefinition<
    typeof AuthEventTypes.Deauthenticated,
    never
  >;
};

export type Event<T extends EventType> = EventTypeMap[T];

export type EventType = keyof EventTypeMap;

export type EventHandler<T extends EventType> = (event: Event<T>) => void;

type InnerEventHandler<T extends EventType> = {
  id: number;
  handler: EventHandler<T>;
};

export class AuthManagerService {
  private subscriberId: number = 0;
  private isProcessing: boolean = false;
  private buffer: Event<EventType>[];
  private subscribers: {
    [K in EventType]: Map<number, InnerEventHandler<K>>;
  };
  private subscribersTypeMap: Map<number, EventType>;
  private lastEvents: {
    [K in EventType]?: Event<K>;
  };

  constructor() {
    this.buffer = [];
    this.subscribers = {
      [AuthEventTypes.Authenticated]: new Map(),
      [AuthEventTypes.Deauthenticated]: new Map(),
    };
    this.lastEvents = {};
    this.subscribersTypeMap = new Map();
  }

  subscribe<T extends EventType>(type: T, handler: EventHandler<T>): number {
    const id = this.subscriberId++;
    this.subscribers[type].set(id, {
      id,
      handler,
    });
    this.subscribersTypeMap.set(id, type);
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

  addEvent(event: Event<EventType>): void {
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

  private setLastEvent<T extends EventType>(type: T, event: Event<T>): void {
    this.lastEvents[type] = event;
  }

  private broadcastEvent<T extends EventType>(type: T, event: Event<T>): void {
    const subscribers = this.subscribers[type].values();
    for (const subscriber of subscribers) {
      subscriber.handler(event);
    }
  }
}
