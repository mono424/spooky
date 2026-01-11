import { Logger } from '../logger/index.js';
import { MutationEventSystem, MutationEventTypes } from '../mutation/events.js';
import { QueryEventSystem, QueryEventTypes } from '../query/events.js';
import { SpookySync, SyncEventTypes, UpEvent } from '../sync/index.js';
import { QueryManager } from '../query/query.js';
import { StreamProcessorService } from '../stream-processor/index.js';
import { DevToolsService } from '../devtools-service/index.js';
import { AuthService, AuthEventTypes } from '../auth/index.js';
import { SchemaStructure } from '@spooky/query-builder';

interface RouteDefinition {
  source: string;
  event: string;
  description: string;
  handler: (payload: any) => Promise<void> | void;
}

export class RouterService<S extends SchemaStructure> {
  constructor(
    private mutationEvents: MutationEventSystem,
    private queryEvents: QueryEventSystem,
    private sync: SpookySync<S>,
    private queryManager: QueryManager<S>,
    private streamProcessor: StreamProcessorService,
    private devTools: DevToolsService,
    private auth: AuthService<S>,
    private logger: Logger
  ) {
    this.logger = logger.child({ service: 'RouterService' });
    this.init();
  }

  private init() {
    this.logger.debug('[RouterService] Initialized with Strict Routing');
    this.registerRoutes();
  }

  private registerRoutes() {
    // -------------------------------------------------------------------------
    // ROUTING TABLE
    // -------------------------------------------------------------------------
    // All cross-service communication is defined here.
    // Sources: Mutation, Query, Sync
    // Targets: Sync, Query, DevTools
    // -------------------------------------------------------------------------

    const routes: RouteDefinition[] = [];

    // --- Mutation Events ---

    routes.push({
      source: 'Mutation',
      event: MutationEventTypes.MutationCreated,
      description: 'Notify DevTools about new mutation',
      handler: (payload) => {
        this.devTools.onMutation(payload);
      },
    });

    routes.push({
      source: 'Mutation',
      event: MutationEventTypes.MutationCreated,
      description: 'Enqueue Mutation in Sync Service',
      handler: (payload) => {
        this.sync.enqueueMutation(payload);
      },
    });

    routes.push({
      source: 'Mutation',
      event: MutationEventTypes.MutationCreated,
      description: 'Ingest Mutation into StreamProcessor',
      handler: (payload: UpEvent[]) => {
        for (const element of payload) {
          const tableName = element.record_id.table.toString();
          this.streamProcessor.ingest(
            tableName,
            element.type,
            element.record_id.toString(),
            element.type === 'delete' ? undefined : element.data
          );
        }
      },
    });

    // --- Query Events ---

    routes.push({
      source: 'Query',
      event: QueryEventTypes.IncantationInitialized,
      description: 'Notify DevTools and Register in Sync',
      handler: (payload) => {
        this.devTools.onQueryInitialized(payload);
        this.sync.enqueueDownEvent({ type: 'register', payload });
        this.streamProcessor.registerIncantation(
          payload.surrealql,
          payload.params,
          payload.incantationId.toString()
        );
      },
    });

    routes.push({
      source: 'Query',
      event: QueryEventTypes.IncantationRemoteHashUpdate,
      description: 'Notify Sync about remote hash update (from Live Query)',
      handler: (payload) => {
        this.devTools.logEvent('QUERY_REMOTE_HASH_UPDATE', payload);
        this.sync.enqueueDownEvent({ type: 'sync', payload });
      },
    });

    routes.push({
      source: 'Query',
      event: QueryEventTypes.IncantationTTLHeartbeat,
      description: 'Send Heartbeat to Sync',
      handler: (payload) => {
        this.devTools.logEvent('QUERY_TTL_HEARTBEAT', payload);
        this.sync.enqueueDownEvent({ type: 'heartbeat', payload });
      },
    });

    routes.push({
      source: 'Query',
      event: QueryEventTypes.IncantationCleanup,
      description: 'Cleanup in Sync',
      handler: (payload) => {
        this.devTools.logEvent('QUERY_CLEANUP', payload);
        this.sync.enqueueDownEvent({ type: 'cleanup', payload });
      },
    });

    routes.push({
      source: 'Query',
      event: QueryEventTypes.IncantationUpdated,
      description: 'Notify DevTools about query update',
      handler: (payload) => {
        this.devTools.onQueryUpdated(payload);
      },
    });

    // --- Sync Events ---

    routes.push({
      source: 'Sync',
      event: SyncEventTypes.IncantationUpdated,
      description: 'Notify QueryManager about new data from remote (via Sync)',
      handler: (payload) => {
        this.devTools.logEvent('SYNC_INCANTATION_UPDATED', payload);
        this.queryManager.handleIncomingUpdate(payload);
      },
    });

    // --- StreamProcessor Events ---

    routes.push({
      source: 'StreamProcessor',
      event: 'stream_update',
      description: 'Notify DevTools about stream processor updates',
      handler: (payload) => {
        this.devTools.onStreamUpdate(payload);
        // Payload is StreamUpdate[]
        if (Array.isArray(payload)) {
          for (const update of payload) {
            this.queryManager.handleStreamUpdate(update);
          }
        }
      },
    });

    // -------------------------------------------------------------------------
    // Execution
    // -------------------------------------------------------------------------

    // Hook up the routes

    // Mutation
    this.mutationEvents.subscribe(MutationEventTypes.MutationCreated, (e) =>
      this.executeRoutes(routes, 'Mutation', MutationEventTypes.MutationCreated, e.payload)
    );

    // Query
    this.queryEvents.subscribe(QueryEventTypes.IncantationInitialized, (e) =>
      this.executeRoutes(routes, 'Query', QueryEventTypes.IncantationInitialized, e.payload)
    );
    this.queryEvents.subscribe(QueryEventTypes.IncantationRemoteHashUpdate, (e) =>
      this.executeRoutes(routes, 'Query', QueryEventTypes.IncantationRemoteHashUpdate, e.payload)
    );
    this.queryEvents.subscribe(QueryEventTypes.IncantationTTLHeartbeat, (e) =>
      this.executeRoutes(routes, 'Query', QueryEventTypes.IncantationTTLHeartbeat, e.payload)
    );
    this.queryEvents.subscribe(QueryEventTypes.IncantationCleanup, (e) =>
      this.executeRoutes(routes, 'Query', QueryEventTypes.IncantationCleanup, e.payload)
    );
    this.queryEvents.subscribe(QueryEventTypes.IncantationUpdated, (e) =>
      this.executeRoutes(routes, 'Query', QueryEventTypes.IncantationUpdated, e.payload)
    );

    // Sync
    this.sync.events.subscribe(SyncEventTypes.IncantationUpdated, (e: any) =>
      this.executeRoutes(routes, 'Sync', SyncEventTypes.IncantationUpdated, e.payload)
    );
    this.sync.events.subscribe(SyncEventTypes.RemoteDataIngested, (e: any) =>
      this.executeRoutes(routes, 'Sync', SyncEventTypes.RemoteDataIngested, e.payload)
    );

    // StreamProcessor
    this.streamProcessor.events.subscribe('stream_update', (e: any) =>
      this.executeRoutes(routes, 'StreamProcessor', 'stream_update', e.payload)
    );

    // Auth
    this.auth.eventSystem.subscribe(AuthEventTypes.AuthStateChanged, (e) => {
      this.executeRoutes(routes, 'Auth', AuthEventTypes.AuthStateChanged, e.payload);
    });
  }

  private executeRoutes(allRoutes: RouteDefinition[], source: string, event: string, payload: any) {
    const matching = allRoutes.filter((r) => r.source === source && r.event === event);
    for (const route of matching) {
      // this.logger.trace({ source, event, desc: route.description }, 'Executing Route');
      try {
        const result = route.handler(payload);
        if (result instanceof Promise) {
          result.catch((err) => this.logger.error({ err, source, event }, 'Route handler failed'));
        }
      } catch (err: any) {
        this.logger.error(
          {
            error: err,
            message: err?.message,
            stack: err?.stack,
            source,
            event,
            payloadPreview: JSON.stringify(payload).substring(0, 200),
          },
          'Route handler failed synchronously'
        );
      }
    }
  }
}
