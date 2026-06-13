import type { RemoteDatabaseService } from '../../services/database/remote';
import type {
  SchemaStructure,
  AccessDefinition,
  ColumnSchema,
  TypeNameToTypeMap,
} from '@spooky-sync/query-builder';
import type { Logger } from '../../services/logger/index';
export * from './events/index';
import { AuthEventTypes, createAuthEventSystem } from './events/index';
import type { PersistenceClient } from '../../types';

// Helper to pretty print types
type Prettify<T> = {
  [K in keyof T]: T[K];
} & {};

// Map ColumnSchema (value type string) to actual Typescript type
type MapColumnType<T extends ColumnSchema> = T['optional'] extends true
  ? TypeNameToTypeMap[T['type']] | undefined
  : TypeNameToTypeMap[T['type']];

// Extract params object from SchemaStructure based on access name and method (signIn/signup)
type ExtractAccessParams<
  S extends SchemaStructure,
  Name extends keyof S['access'],
  Method extends 'signIn' | 'signup',
> = S['access'] extends undefined
  ? never
  : S['access'][Name] extends AccessDefinition
    ? Prettify<{
        [K in keyof S['access'][Name][Method]['params']]: MapColumnType<
          S['access'][Name][Method]['params'][K]
        >;
      }>
    : never;

/**
 * Read the `AC` (access-method name) claim from a SurrealDB record-access
 * JWT without verifying it — we only need the claim, the server enforces the
 * token. Returns null on any malformed input. The in-browser SSP needs this
 * to resolve `$access` in table permission predicates (mirrors the session's
 * `$access` that the server's `fn::query::register` reads).
 */
function decodeAccessFromToken(token: string): string | null {
  try {
    const payload = token.split('.')[1];
    if (!payload) return null;
    let b64 = payload.replace(/-/g, '+').replace(/_/g, '/');
    b64 += '='.repeat((4 - (b64.length % 4)) % 4);
    const json =
      typeof atob === 'function' ? atob(b64) : Buffer.from(b64, 'base64').toString('binary');
    const claims = JSON.parse(json) as Record<string, unknown>;
    const ac = claims.AC ?? claims.ac;
    return typeof ac === 'string' ? ac : null;
  } catch {
    return null;
  }
}

export class AuthService<S extends SchemaStructure> {
  // State
  public token: string | null = null;
  public currentUser: any | null = null;
  public isAuthenticated: boolean = false;
  /**
   * The record-access method name for the current session (e.g. `"account"`),
   * derived from the token's `AC` claim. Consumed by the in-browser SSP's
   * permission injection so `$access`-gated table predicates resolve locally,
   * mirroring the server's `$access`. Null when logged out.
   */
  public access: string | null = null;
  public isLoading: boolean = true;

  private events = createAuthEventSystem();

  public get eventSystem() {
    return this.events;
  }

  constructor(
    private schema: S,
    private remote: RemoteDatabaseService,
    private persistenceClient: PersistenceClient,
    private logger: Logger
  ) {}

  async init() {
    await this.check();
  }

  getAccessDefinition<Name extends keyof S['access']>(name: Name): AccessDefinition | undefined {
    return this.schema.access?.[name as string];
  }

  /**
   * Subscribe to auth state changes.
   * callback is called immediately with current value and whenever validation status changes.
   */
  subscribe(cb: (userId: string | null) => void): () => void {
    // Immediate callback
    cb(this.currentUser?.id || null);

    const id = this.events.subscribe(AuthEventTypes.AuthStateChanged, (event) => {
      cb(event.payload);
    });

    return () => {
      this.events.unsubscribe(id);
    };
  }

  private notifyListeners() {
    const userId = this.currentUser?.id || null;
    this.events.emit(AuthEventTypes.AuthStateChanged, userId);
  }

  /**
   * Check for existing session and validate
   */
  async check(accessToken?: string) {
    this.isLoading = true;

    try {
      const token = accessToken || (await this.persistenceClient.get<string>('sp00ky_auth_token'));

      if (!token) {
        this.logger.debug(
          { Category: 'sp00ky-client::AuthService::check' },
          'No token found in storage or arguments'
        );
        this.isLoading = false;
        this.isAuthenticated = false;
        this.notifyListeners();
        return;
      }

      // Authenticate with the token
      await this.remote.getClient().authenticate(token);

      // Verify the session by fetching the full user record using $auth.id
      const result = await this.remote.query('SELECT * FROM ONLY $auth.id');

      const items = Array.isArray(result) && Array.isArray(result[0]) ? result[0] : result;
      const user = Array.isArray(items) ? items[0] : items;

      if (user && user.id) {
        this.logger.info(
          { user, Category: 'sp00ky-client::AuthService::check' },
          'Auth check complete (via $auth.id)'
        );
        await this.setSession(token, user);
      } else {
        this.logger.warn(
          { Category: 'sp00ky-client::AuthService::check' },
          '$auth.id empty, attempting manual user fetch'
        );

        const manualResult = await this.remote.query(
          'SELECT * FROM user WHERE id = $auth.id LIMIT 1'
        );
        const manualItems =
          Array.isArray(manualResult) && Array.isArray(manualResult[0])
            ? manualResult[0]
            : manualResult;
        const manualUser = Array.isArray(manualItems) ? manualItems[0] : manualItems;

        if (manualUser && manualUser.id) {
          this.logger.info(
            { user: manualUser, Category: 'sp00ky-client::AuthService::check' },
            'Auth check complete (via manual fetch)'
          );
          await this.setSession(token, manualUser);
        } else {
          this.logger.warn(
            { Category: 'sp00ky-client::AuthService::check' },
            'Token valid but user not found via fallback'
          );
          await this.signOut();
        }
      }
    } catch (error) {
      this.logger.error(
        { error, stack: (error as Error).stack, Category: 'sp00ky-client::AuthService::check' },
        'Auth check failed'
      );
      await this.signOut();
    } finally {
      this.isLoading = false;
    }
  }

  /**
   * Sign out and clear session
   */
  async signOut() {
    this.token = null;
    this.currentUser = null;
    this.isAuthenticated = false;
    this.access = null;

    await this.persistenceClient.remove('sp00ky_auth_token');

    try {
      await this.remote.getClient().invalidate();
    } catch (_e) {
      // Ignore invalidation errors
    }

    this.notifyListeners();
  }

  private async setSession(token: string, user: any) {
    this.token = token;
    this.currentUser = user;
    this.isAuthenticated = true;
    // Resolve the access-method name (e.g. "account") for in-browser SSP
    // permission injection. Prefer the token's `AC` claim; fall back to the
    // schema's sole record-access method if the claim is absent.
    this.access = decodeAccessFromToken(token) ?? this.defaultAccessName();
    await this.persistenceClient.set('sp00ky_auth_token', token);
    this.notifyListeners();
  }

  /** Fallback when the token carries no `AC` claim: if the schema defines
   *  exactly one record-access method, assume the session used it. */
  private defaultAccessName(): string | null {
    const names = Object.keys(this.schema.access ?? {});
    return names.length === 1 ? names[0] : null;
  }

  async signUp<Name extends keyof S['access'] & string>(
    accessName: Name,
    params: ExtractAccessParams<S, Name, 'signup'>
  ) {
    const def = this.getAccessDefinition(accessName);
    if (!def) throw new Error(`Access definition '${accessName}' not found`);

    // Verify all required params are present
    // Safe cast params to Record<string, any> for runtime check
    const runtimeParams = params as Record<string, any>;

    const missingParams = Object.entries(def.signup.params)
      .filter(([name, schema]) => !schema.optional && !(name in runtimeParams))
      .map(([name]) => name);

    if (missingParams.length > 0) {
      throw new Error(
        `Missing required signup params for '${accessName}': ${missingParams.join(', ')}`
      );
    }

    this.logger.info(
      { accessName, runtimeParams, Category: 'sp00ky-client::AuthService::signUp' },
      'Attempting signup'
    );

    const { access } = await this.remote.getClient().signup({
      access: accessName,
      variables: runtimeParams,
    });

    this.logger.info(
      { Category: 'sp00ky-client::AuthService::signUp' },
      'Signup successful, token received'
    );

    // After signup, we usually get a token.
    // We should also fetch the user or trust the token works.
    // For now, let's just trigger a check() to fully hydrate state
    await this.check(access);
  }

  async signIn<Name extends keyof S['access'] & string>(
    accessName: Name,
    params: ExtractAccessParams<S, Name, 'signIn'>
  ) {
    const def = this.getAccessDefinition(accessName);
    if (!def) throw new Error(`Access definition '${accessName}' not found`);

    const runtimeParams = params as Record<string, any>;

    // Verify all required params are present
    const missingParams = Object.entries(def.signIn.params)
      .filter(([name, schema]) => !schema.optional && !(name in runtimeParams))
      .map(([name]) => name);

    if (missingParams.length > 0) {
      throw new Error(
        `Missing required signin params for '${accessName}': ${missingParams.join(', ')}`
      );
    }

    this.logger.info(
      { accessName, Category: 'sp00ky-client::AuthService::signIn' },
      'Attempting signin'
    );

    const { access } = await this.remote.getClient().signin({
      access: accessName,
      variables: runtimeParams,
    });

    await this.check(access);
  }
}
