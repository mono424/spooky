# Secrets in migrations (vault placeholder substitution)

Some schema needs a secret value that must **not** be committed — e.g. an
asymmetric JWT signing key for a record-access definition:

```surql
DEFINE ACCESS account ON DATABASE TYPE RECORD
  SIGNUP {...} SIGNIN (...)
  WITH JWT ALGORITHM EDDSA KEY '<public PEM>' WITH ISSUER KEY '<private PEM>'
  DURATION FOR TOKEN 365d, FOR SESSION 365d;
```

Migrations are applied **verbatim**, so you can't paste the real key into the
schema without it landing in git. Instead, write a placeholder and let the CLI
inject the real value from the vault at apply time.

## Placeholders

Use `{{UPPER_SNAKE}}` tokens in your schema / migrations:

```surql
WITH JWT ALGORITHM EDDSA KEY '{{JWT_PUBLIC_KEY}}' WITH ISSUER KEY '{{JWT_PRIVATE_KEY}}'
```

- Only `{{UPPER_SNAKE}}` tokens are treated as placeholders — ordinary text or
  comments with braces are left untouched.
- At apply time, each `{{KEY}}` is replaced with the matching vault secret.
- **Any unresolved `{{KEY}}` fails the apply loudly** — a missing secret never
  writes a broken value.

## Where the values come from

The substitution source is the project **vault** (the same secrets backends
read). Set a value — including multi-line ones like PEM keys — from a file:

```sh
spky env set JWT_PUBLIC_KEY  --file jwt_ed25519_pub.pem
spky env set JWT_PRIVATE_KEY --file jwt_ed25519_priv.pem
# scope: defaults to both environments; use --dev or --prod to target one
```

`spky env set` also still prompts for the value interactively when `--file` is
omitted.

## Coverage

- **Wired:** `spky dev` (the default **legacy** migration engine). It loads the
  development vault and substitutes `{{KEY}}` during the user-migration apply.
- **Ephemeral schema diff** (`spky generate`) replays migrations with
  placeholders left **literal**, so old/new schemas match and a `{{KEY}}` never
  reads as drift.
- **Migration checksums** are computed on the on-disk file (pre-substitution),
  so they stay stable regardless of secret values or machine.
- **Cloud deploy:** not yet wired — set `MigrationContext.secrets` at the deploy
  call site to enable substitution there.
- **surrealkit engine:** not supported; the CLI warns if secrets are present.

## Implementation

- `apps/cli/src/migrate.rs` — `substitute_secrets` (+ unresolved-placeholder
  guard) and `apply_with_secrets`. `apply` stays literal (used by the diff).
- `apps/cli/src/migration/legacy.rs` — `LegacyEngine` calls `apply_with_secrets`
  when `MigrationContext.secrets` is non-empty.
- `apps/cli/src/cloud.rs` — `env set --file` reads the value from a file.
