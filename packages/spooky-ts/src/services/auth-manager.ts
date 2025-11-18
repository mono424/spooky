import { DatabaseService } from "./index.js";
import type { RecordId } from "surrealdb";

export class AuthManagerService {
  private token: string = "";
  private userId: RecordId | undefined = undefined;

  constructor(private databaseService: DatabaseService) {}

  getToken(): string {
    return this.token;
  }

  getUserId(): RecordId | undefined {
    return this.userId;
  }

  async reauthenticate(): Promise<void> {
    if (!this.token) {
      throw new Error("No token found");
    }
    try {
      await this.authenticate(this.token);
    } catch (error) {
      this.deauthenticate();
      throw new Error("Failed to reauthenticate", { cause: error });
    }
  }

  async authenticate(token: string): Promise<RecordId | undefined> {
    const userId = await this.databaseService.authenticate(token);
    this.token = token;
    this.userId = userId;
    return userId;
  }

  async deauthenticate(): Promise<void> {
    this.token = "";
    this.userId = undefined;
    await this.databaseService.deauthenticate();
  }
}

export function createAuthManagerService(
  databaseService: DatabaseService
): AuthManagerService {
  return new AuthManagerService(databaseService);
}
