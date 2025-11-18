import { AuthEventTypes, DatabaseService, SpookyEventSystem } from "./index.js";

export class AuthManagerService {
  private token: string = "";

  constructor(
    private databaseService: DatabaseService,
    private eventSystem: SpookyEventSystem
  ) {
    eventSystem.subscribe(AuthEventTypes.Authenticated, (event) => {
      this.token = event.payload.token;
    });
    eventSystem.subscribe(AuthEventTypes.Deauthenticated, () => {
      this.token = "";
    });
  }

  async reauthenticate(): Promise<void> {
    if (!this.token) {
      throw new Error("No token found");
    }
    try {
      await this.authenticate(this.token);
    } catch (error) {
      await this.deauthenticate();
      throw new Error("Failed to reauthenticate", { cause: error });
    }
  }

  async authenticate(token: string): Promise<void> {
    const userId = await this.databaseService.authenticate(token);
    if (!userId) {
      await this.deauthenticate();
      throw new Error("Failed to authenticate");
    }
    this.eventSystem.addEvent({
      type: AuthEventTypes.Authenticated,
      payload: {
        userId,
        token,
      },
    });
  }

  async deauthenticate(): Promise<void> {
    await this.databaseService.deauthenticate();
    this.eventSystem.addEvent({
      type: AuthEventTypes.Deauthenticated,
    });
  }
}

export function createAuthManagerService(
  databaseService: DatabaseService,
  eventSystem: SpookyEventSystem
): AuthManagerService {
  return new AuthManagerService(databaseService, eventSystem);
}
