export class SurrealClient {
    config;
    authHeader;
    constructor(config) {
        this.config = config;
        this.authHeader =
            'Basic ' + Buffer.from(`${config.username}:${config.password}`).toString('base64');
    }
    async query(surql) {
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
        return res.json();
    }
}
