import { RemoteDatabaseService } from '../../services/database/remote.js';
import { LocalDatabaseService } from '../../services/database/local.js';
import { DataModule } from '../data/index.js';
import {
  SchemaStructure,
  AccessDefinition,
  ColumnSchema,
  TypeNameToTypeMap,
} from '@spooky/query-builder';
import { Logger } from '../../services/logger/index.js';
import { encodeRecordId } from '../../utils/index.js';
export * from './events/index.js';
import { AuthEventTypes, createAuthEventSystem } from './events/index.js';
import { PersistenceClient } from '../../types.js';

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

export class AuthService<S extends SchemaStructure> {
  // State
  public token: string | null = null;
  public currentUser: any | null = null;
  public isAuthenticated: boolean = false;
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
      const token = accessToken || (await this.persistenceClient.get<string>('spooky_auth_token'));

      if (!token) {
        this.logger.debug('[AuthService] No token found in storage or arguments');
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
        this.logger.info({ user }, '[AuthService] Auth check complete (via $auth.id)');
        await this.setSession(token, user);
      } else {
        this.logger.warn('[AuthService] $auth.id empty, attempting manual user fetch');

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
            { user: manualUser },
            '[AuthService] Auth check complete (via manual fetch)'
          );
          await this.setSession(token, manualUser);
        } else {
          this.logger.warn('[AuthService] Token valid but user not found via fallback');
          await this.signOut();
        }
      }
    } catch (error) {
      this.logger.error(
        { error, stack: (error as Error).stack },
        '[AuthService] Auth check failed'
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

    await this.persistenceClient.remove('spooky_auth_token');

    try {
      await this.remote.getClient().invalidate();
    } catch (e) {
      // Ignore invalidation errors
    }

    this.notifyListeners();
  }

  private async setSession(token: string, user: any) {
    this.token = token;
    this.currentUser = user;
    this.isAuthenticated = true;
    await this.persistenceClient.set('spooky_auth_token', token);
    this.notifyListeners();
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

    this.logger.info({ accessName, runtimeParams }, '[AuthService] Attempting signup');

    const { access } = await this.remote.getClient().signup({
      access: accessName,
      variables: runtimeParams,
    });

    this.logger.info('[AuthService] Signup successful, token received');

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

    const { access } = await this.remote.getClient().signin({
      access: accessName,
      variables: runtimeParams,
    });

    await this.check(access);
  }
}
