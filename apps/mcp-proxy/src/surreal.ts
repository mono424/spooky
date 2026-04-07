export interface SurrealConfig {
  url: string;
  namespace: string;
  database: string;
  username: string;
  password: string;
}

export class SurrealClient {
  private authHeader: string;

  constructor(private config: SurrealConfig) {
    this.authHeader =
      'Basic ' + Buffer.from(`${config.username}:${config.password}`).toString('base64');
  }

  async query(surql: string): Promise<unknown[]> {
    const res = await fetch(`${this.config.url}/sql`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Authorization: this.authHeader,
        'surreal-ns': this.config.namespace,
        'surreal-db': this.config.database,
        Accept: 'application/json',
      },
      body: surql,
    });

    if (!res.ok) {
      const text = await res.text();
      throw new Error(`SurrealDB query failed (${res.status}): ${text}`);
    }

    return res.json() as Promise<unknown[]>;
  }
}
