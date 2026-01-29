## Register Query Flow

1. Query registering (local) - Data Module
2. Query get local data - Data Module
3. (Query registing (remote) - Sync Module)

## Query Local Updates

1. Local Mutation is executed - Data Module
2. Mutation is enqueued for remote - Sync Moduke
3. New Record is ingested to DBSP - Data Module
4. Related Queries are updated - Data Module
5. (Sync Module syncs the mutation to remote - Sync Module)

## Query Live Updates

1. Live Update is received - Sync Module
2. Version is compared with local version - Sync Module
3. If remote version is higher, the record is updated - Sync Module
4. Record is ingested to DBSP Module - Data Module
5. Related Queries are updated - Data Module
