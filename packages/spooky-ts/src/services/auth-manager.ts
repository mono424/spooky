import { Context, Effect, Layer, Ref } from "effect";
import { SchemaStructure } from "@spooky/query-builder";
import { DatabaseService, RemoteAuthenticationError } from "./index.js";
import { RecordId } from "surrealdb";

export class AuthManagerService extends Context.Tag("AuthManagerService")<
  AuthManagerService,
  {
    readonly getToken: () => Effect.Effect<string>;
    readonly getUserId: () => Effect.Effect<RecordId | undefined>;
    readonly authenticate: (
      token: string
    ) => Effect.Effect<RecordId, RemoteAuthenticationError, never>;
  }
>() {}

export const AuthManagerServiceLayer = <S extends SchemaStructure>() =>
  Layer.scoped(
    AuthManagerService,
    Effect.gen(function* () {
      const databaseService = yield* DatabaseService;
      const tokenRef = yield* Ref.make("");
      const userIdRef = yield* Ref.make<RecordId | undefined>(undefined);

      const authenticate = Effect.fn("authenticate")(function* (token: string) {
        const userId = yield* databaseService.authenticate(token);
        yield* Ref.set(tokenRef, token);
        yield* Ref.set(userIdRef, userId);
        return userId;
      });

      const getToken = () => Ref.get(tokenRef);
      const getUserId = () => Ref.get(userIdRef);

      return AuthManagerService.of({
        getToken,
        getUserId,
        authenticate,
      });
    })
  );
