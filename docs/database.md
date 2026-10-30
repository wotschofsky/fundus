# Checked SQL

SQLx checks static application queries against a local SQLite database when Rust
compiles. There is no generated query cache to maintain.

## Set up the development database

Run these commands from the repository root:

```sh
cp -n .env.example .env
cargo install sqlx-cli --version 0.8.6 --locked --no-default-features --features sqlite
mkdir -p data
cargo sqlx database setup --source crates/server/migrations
cargo check --all-targets
```

The example `.env` sets `DATABASE_URL=sqlite://data/vault.sqlite`. SQLx reads it
automatically. Keep an existing `.env`; only add `DATABASE_URL` if it is missing.
The database and environment file are ignored by Git.

This is also the default database used when running the server locally. It starts
empty, with the schema applied, and needs no recordings, provider API keys, or
production access for compilation. The application binary still requires its
runtime configuration to be exported in the shell as described in the setup guide.

Once the database exists, use normal Rust commands:

```sh
cargo check --all-targets
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
```

## Change the schema

Add a migration, apply it locally, and compile:

```sh
cargo sqlx migrate run --source crates/server/migrations
cargo check --all-targets
```

The build watches the migrations and default development database so schema
changes trigger another query check. For a custom `DATABASE_URL`, changing that
environment variable also triggers a rebuild; use `cargo clean -p fundus` after
changing only the schema of a database outside the project’s default data path.

CI and Docker create fresh databases from the migrations before compiling. A query
that works only because of an unrecorded local schema change will fail there.
The Docker build creates its own empty database; it never copies your local data
or `.env` into the image. The database and migration CLI stay in the build stage.

## Query arguments and SQLite types

Static queries use `query!` and `query_scalar!`. SQLx checks SQL syntax, referenced
tables and columns, argument counts, and result mappings during compilation.

SQLite needs explicit type or nullability annotations for some computed values.
For example, `AS "id!"` declares a non-null result and `AS "exists!: bool"`
also supplies its Rust type. These annotations must follow the query and
application invariants; SQLite’s `TEXT PRIMARY KEY` alone does not imply
`NOT NULL`. Fundus supplies non-null IDs; optional fields remain `Option<T>`.

Simple Unix timestamps use SQLite’s `unixepoch()` directly in SQL. SQLx 0.8
borrows macro arguments, so generated strings and other temporary Rust values
need named locals when their lifetimes must span the asynchronous query.

## Limits

Dynamic library filters, search queries, and variable-length bind lists remain
runtime queries covered by integration tests. Test-fixture SQL is exercised by
those tests.

Compilation checks the development schema, not a live deployment. Keep migrations
and behavioral tests; query checking does not prove business logic or audit
manual production schema changes. Never edit an already-applied migration to
change a deployed schema.
