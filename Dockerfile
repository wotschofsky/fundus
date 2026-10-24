FROM node:26-bookworm-slim AS web
WORKDIR /src
# Node 26 does not bundle Corepack; npm is used only to bootstrap the pinned PNPM CLI.
RUN npm install --global pnpm@10.34.5
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml .npmrc ./
COPY web/package.json web/package.json
COPY apps/mobile/package.json apps/mobile/package.json
RUN --mount=type=cache,target=/pnpm-store pnpm --filter transcript-vault-web install --frozen-lockfile --config.node-linker=isolated --store-dir=/pnpm-store
COPY web/ web/
RUN pnpm --dir web build

FROM python:3.12-slim-bookworm AS runtime-deps
RUN pip install --no-cache-dir --target=/runtime onnxruntime==1.31.0

FROM rust:1.94-bookworm AS server
WORKDIR /src
ARG CARGO_BUILD_JOBS=2
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
COPY --from=web /src/web/dist web/dist/
RUN --mount=type=cache,target=/usr/local/cargo/registry --mount=type=cache,target=/src/target cargo build --locked --release --jobs "$CARGO_BUILD_JOBS" -p transcript-vault && cp target/release/transcript-vault /usr/local/bin/transcript-vault
COPY scripts/fetch-speaker-model.sh /tmp/fetch-speaker-model.sh
RUN VAULT_MODEL_DIR=/models sh /tmp/fetch-speaker-model.sh

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates ffmpeg libgomp1 && rm -rf /var/lib/apt/lists/* && useradd --system --uid 10001 --create-home vault && mkdir /data && chown vault:vault /data
COPY --from=server /usr/local/bin/transcript-vault /usr/local/bin/transcript-vault
COPY --from=server /models /opt/vault/models
COPY --from=runtime-deps /runtime/onnxruntime/capi/libonnxruntime.so.1.31.0 /usr/local/lib/libonnxruntime.so.1.31.0
ENV VAULT_BIND=0.0.0.0:3000 VAULT_DATA_DIR=/data VAULT_VOICE_MODEL=/opt/vault/models/speaker-embedding.onnx ORT_DYLIB_PATH=/usr/local/lib/libonnxruntime.so.1.31.0
USER vault
EXPOSE 3000
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s CMD ["transcript-vault", "healthcheck"]
ENTRYPOINT ["transcript-vault"]
CMD ["serve"]
