import { DatabaseService } from "./database.js";

export class AuthManager {
  constructor(private db: DatabaseService) {}

  async authenticate(token: string): Promise<void> {
    await this.db.getRemote().authenticate(token);
  }

  async deauthenticate(): Promise<void> {
    await this.db.getRemote().invalidate();
  }
}
