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

FROM buildpack-deps:bookworm-curl AS model
COPY scripts/fetch-speaker-model.sh /tmp/fetch-speaker-model.sh
RUN VAULT_MODEL_DIR=/models sh /tmp/fetch-speaker-model.sh

# Only what crates/server/src/audio.rs and pipeline.rs run: common recorder and phone formats in
# (WAV, AIFF, CAF, FLAC, MP3, M4A with AAC or ALAC, Ogg and WebM with Opus or Vorbis); WAV, FLAC,
# M4A/AAC and raw PCM out.
# Debian's ffmpeg package would add ~390 MB of codecs, devices and X11/SDL libraries.
FROM buildpack-deps:bookworm AS ffmpeg
RUN apt-get update && apt-get install -y --no-install-recommends nasm && rm -rf /var/lib/apt/lists/*
# Release tarball signed by the FFmpeg release key FCF9 86EA 15E6 E293 A564 4F10 B432 2F04 D676 58D8.
ARG FFMPEG_VERSION=8.1.3
ARG FFMPEG_SHA256=7138d28c96d9d3e3af4ee3d8cad72741f8ffb40da90c1112235dea3ecd3178a3
WORKDIR /ffmpeg
RUN curl --fail --silent --show-error --location --retry 3 "https://ffmpeg.org/releases/ffmpeg-$FFMPEG_VERSION.tar.xz" -o /tmp/ffmpeg.tar.xz \
    && echo "$FFMPEG_SHA256  /tmp/ffmpeg.tar.xz" | sha256sum -c - \
    && tar -xJf /tmp/ffmpeg.tar.xz --strip-components=1 \
    && ./configure --disable-autodetect --disable-everything --disable-doc --disable-debug \
        --disable-network --disable-avdevice --disable-swscale \
        --enable-protocol=file,pipe \
        --enable-demuxer=wav,aiff,caf,flac,mp3,aac,mov,ogg,matroska \
        --enable-parser=flac,mpegaudio,aac,opus,vorbis \
        --enable-decoder=pcm_u8,pcm_s8,pcm_s16le,pcm_s16be,pcm_s24le,pcm_s24be,pcm_s32le,pcm_s32be,pcm_f32le,pcm_f32be,pcm_f64le,pcm_f64be,flac,mp3float,mp3,aac,alac,opus,vorbis \
        --enable-encoder=pcm_s16le,pcm_s32le,pcm_f32le,pcm_f64le,flac,aac \
        --enable-muxer=wav,flac,ipod,pcm_s32le,pcm_f32le,pcm_f64le \
        --enable-filter=aformat,anull,aresample,asetpts,asplit,atrim,concat \
    && make -j"$(nproc)" \
    && install -D -m 755 -t /opt/ffmpeg ffmpeg ffprobe

# The web app is served from /opt/vault/web at runtime, so UI changes never recompile the server.
FROM rust:1.94-bookworm AS server
WORKDIR /src
ARG CARGO_BUILD_JOBS=2
COPY Cargo.toml Cargo.lock ./
COPY crates/ crates/
RUN --mount=type=cache,target=/usr/local/cargo/registry --mount=type=cache,target=/src/target cargo build --locked --release --jobs "$CARGO_BUILD_JOBS" -p transcript-vault && cp target/release/transcript-vault /usr/local/bin/transcript-vault

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates libgomp1 && rm -rf /var/lib/apt/lists/* && useradd --system --uid 10001 --create-home vault && mkdir /data && chown vault:vault /data
COPY --from=ffmpeg /opt/ffmpeg/ /usr/local/bin/
COPY --from=runtime-deps /runtime/onnxruntime/capi/libonnxruntime.so.1.31.0 /usr/local/lib/libonnxruntime.so.1.31.0
COPY --from=model /models /opt/vault/models
COPY --from=server /usr/local/bin/transcript-vault /usr/local/bin/transcript-vault
COPY --from=web /src/web/dist /opt/vault/web
ENV VAULT_BIND=0.0.0.0:3000 VAULT_DATA_DIR=/data VAULT_WEB_DIR=/opt/vault/web VAULT_VOICE_MODEL=/opt/vault/models/speaker-embedding.onnx ORT_DYLIB_PATH=/usr/local/lib/libonnxruntime.so.1.31.0
USER vault
EXPOSE 3000
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s CMD ["transcript-vault", "healthcheck"]
ENTRYPOINT ["transcript-vault"]
CMD ["serve"]
