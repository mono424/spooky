import { onCleanup, createEffect, Accessor, createSignal } from "solid-js";
import type {
  ColumnSchema,
  FinalQuery,
  TableModel,
  SchemaStructure,
  TableNames,
  GetTable,
} from "@spooky/query-builder";

// Helper type to extract all generic parameters from FinalQuery
type ExtractFinalQueryParams<T> = T extends FinalQuery<
  infer S,
  infer TableName,
  infer Table,
  infer RelatedFields,
  infer IsOne
>
  ? {
      S: S;
      TableName: TableName;
      Table: Table;
      RelatedFields: RelatedFields;
      IsOne: IsOne;
    }
  : never;

// Helper type to get the related model type for a table
type GetRelatedModel<
  S extends SchemaStructure,
  RelatedTableName extends string
> = RelatedTableName extends TableNames<S>
  ? TableModel<GetTable<S, RelatedTableName>>
  : never;

// Helper type to build the related fields object based on accumulated relationships
type BuildRelatedFields<
  S extends SchemaStructure,
  TableName extends TableNames<S>,
  RelatedFields extends Record<string, any>
> = {
  [K in keyof RelatedFields]: RelatedFields[K] extends { cardinality: "one" }
    ? GetRelatedModel<S, RelatedFields[K]["to"]> | null
    : GetRelatedModel<S, RelatedFields[K]["to"]>[];
};

// The final result type combining base model with related fields
type QueryResultType<
  S extends SchemaStructure,
  TableName extends TableNames<S>,
  Table extends { columns: Record<string, ColumnSchema> },
  RelatedFields extends Record<string, any>,
  IsOne extends boolean
> = IsOne extends true
  ?
      | (TableModel<Table> & BuildRelatedFields<S, TableName, RelatedFields>)
      | undefined
  : (TableModel<Table> & BuildRelatedFields<S, TableName, RelatedFields>)[];

// Conditional return type based on IsOne
type UseQueryReturn<T extends FinalQuery<any, any, any, any, any>> =
  ExtractFinalQueryParams<T> extends {
    S: infer S;
    TableName: infer TableName;
    Table: infer Table;
    RelatedFields: infer RelatedFields;
    IsOne: infer IsOne;
  }
    ? S extends SchemaStructure
      ? TableName extends TableNames<S>
        ? Table extends { columns: Record<string, ColumnSchema> }
          ? RelatedFields extends Record<string, any>
            ? IsOne extends boolean
              ? Accessor<
                  QueryResultType<S, TableName, Table, RelatedFields, IsOne>
                >
              : never
            : never
          : never
        : never
      : never
    : never;

// Single signature with conditional return type
export function useQuery<T extends FinalQuery<any, any, any, any, any>>(
  queryResult: Accessor<T>
): [UseQueryReturn<T>] {
  type Params = ExtractFinalQueryParams<T>;
  type S = Params["S"];
  type TableName = Params["TableName"];
  type Table = Params["Table"];
  type RelatedFields = Params["RelatedFields"];
  type ResultType = QueryResultType<S, TableName, Table, RelatedFields, false>;

  // Create internal signal to store data (always as array internally)
  const [data, setData] = createSignal<ResultType>([]);

  // Track the previous query to detect changes
  let previousQueryHash: number | null = null;

  // Track the current query and cleanup
  createEffect(() => {
    const query = queryResult();
    if (query.hash === previousQueryHash) return;
    previousQueryHash = query.hash;

    const { data, subscribe } = query.select();
    setData(() => data as ResultType);

    const unsubscribe = subscribe((newData) =>
      setData(() => newData as ResultType)
    );
    onCleanup(() => unsubscribe());
  });

  // Check if query is a "one" query once
  const isOneQuery = queryResult().isOne;

  // Return either single item or array based on isOne flag
  if (isOneQuery) {
    return [(() => data()[0]) as UseQueryReturn<T>];
  }

  return [data as UseQueryReturn<T>];
}
