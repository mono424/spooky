export interface SurrealConfig {
    url: string;
    namespace: string;
    database: string;
    username: string;
    password: string;
}
export declare class SurrealClient {
    private config;
    private authHeader;
    constructor(config: SurrealConfig);
    query(surql: string): Promise<unknown[]>;
}
