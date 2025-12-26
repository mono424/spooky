import { LocalDatabaseService } from "../../database/index.js"
import { createSyncQueueEventSystem, SyncQueueEventSystem, SyncQueueEventTypes } from "../events.js"
import { QueryEventSystem, QueryEventTypeMap, QueryEventTypes } from "../../query/events.js"
import { EventPayload } from "../../../events/index.js"

export type RegisterEvent = {
    type: "register",
    payload: EventPayload<QueryEventTypeMap, "QUERY_INCANTATION_INITIALIZED">
}

export type SyncEvent = {
    type: "sync",
    payload: EventPayload<QueryEventTypeMap, "QUERY_INCANTATION_REMOTE_HASH_UPDATE">
}

export type HeartbeatEvent = {
    type: "heartbeat",
    payload: EventPayload<QueryEventTypeMap, "QUERY_INCANTATION_TTL_HEARTBEAT">
}

export type CleanupEvent = {
    type: "cleanup",
    payload: EventPayload<QueryEventTypeMap, "QUERY_INCANTATION_CLEANUP">
}

export type DownEvent = RegisterEvent | SyncEvent | HeartbeatEvent | CleanupEvent;

export class DownQueue {
    private queue: DownEvent[] = [];
    private _events: SyncQueueEventSystem;

    get events(): SyncQueueEventSystem {
        return this._events;
    }

    constructor(private local: LocalDatabaseService) {
        this._events = createSyncQueueEventSystem();
    }

    get size(): number {
        return this.queue.length;
    }

    push(event: DownEvent) {
        this.queue.push(event);
        this.emitPushEvent(event)
    }

    private emitPushEvent(event: DownEvent) {
        switch (event.type) {
            case "register":
                this._events.addEvent({
                    type: SyncQueueEventTypes.IncantationRegistrationEnqueued,
                    payload: {
                        incantationId: event.payload.incantationId,
                        surrealql: event.payload.surrealql,
                        ttl: event.payload.ttl,
                    }
                });
                break;
            case "sync":
                this._events.addEvent({
                    type: SyncQueueEventTypes.IncantationSyncEnqueued,
                    payload: {
                        incantationId: event.payload.incantationId,
                        remoteHash: event.payload.remoteHash,
                    }
                });
                break;
            case "heartbeat":
                this._events.addEvent({
                    type: SyncQueueEventTypes.IncantationTTLHeartbeatEnqueued,
                    payload: {
                        incantationId: event.payload.incantationId,
                    }
                });
                break;
            case "cleanup":
                this._events.addEvent({
                    type: SyncQueueEventTypes.IncantationCleanupEnqueued,
                    payload: {
                        incantationId: event.payload.incantationId,
                    }
                });
                break;
        }
    }

    async next(fn: (event: DownEvent) => Promise<void>): Promise<void> {
        const event = this.queue.shift();
        if (event) {
            try {
                await fn(event);
            } catch (error) {
                console.error("Failed to process query", event, error);
                this.queue.unshift(event);
                throw error;
            }
        }
    }

    listenForQueries(queryEvents: QueryEventSystem) {
        queryEvents.subscribe(QueryEventTypes.IncantationInitialized, (event) => {
            this.push({
                type: "register",
                payload: event.payload,
            });
        });
        queryEvents.subscribe(QueryEventTypes.IncantationRemoteHashUpdate, (event) => {
            this.push({
                type: "sync",
                payload: event.payload,
            });
        });
        queryEvents.subscribe(QueryEventTypes.IncantationTTLHeartbeat, (event) => {
            this.push({
                type: "heartbeat",
                payload: event.payload,
            });
        });
        queryEvents.subscribe(QueryEventTypes.IncantationCleanup, (event) => {
            this.push({
                type: "cleanup",
                payload: event.payload,
            });
        });
    }        
}