# Packaging and releases

The server image bundles the web UI, Rust binary, FFmpeg, ONNX Runtime and pinned
speaker model. Node and Python are build dependencies, absent from the final image.
Upload clients remain separate native applications.

FFmpeg is built from the signed release tarball pinned in the `Dockerfile`
(`FFMPEG_VERSION`, `FFMPEG_SHA256`) with only what the server runs: WAV, FLAC, MP3
and M4A (AAC/ALAC) in; WAV, FLAC, M4A and raw PCM out. Other formats fail
verification. The image scan cannot see this build: bump both values when FFmpeg
publishes a security release.

## Container CI

`containers.yml` runs on PRs, pushes to `main`, `v*` tags and manual runs. It calls
`check.yml` for Rust, web, native codec, Swift and mobile TypeScript checks, then
builds and smoke-tests each architecture. Fixable HIGH/CRITICAL scan findings
block publication. PRs never log in or publish.

Native builds use `ubuntu-24.04` and `ubuntu-24.04-arm` runners. Allow package
writes for `GITHUB_TOKEN` in the repository. Substitute compatible native runner
labels if needed. The image path comes from `github.repository`.

Published under `ghcr.io/<owner>/<repository>`:

- `sha-<full commit>-amd64` / `sha-<full commit>-arm64`: tested architectures.
- `sha-<full commit>`: combined image; the manifest job verifies both platforms.
- `latest`: only a successful build of the current `main` tip updates this tag.
- `1.2.3`: a `v1.2.3` tag. SemVer `+` metadata becomes `_` for Docker tags.

Published images include SBOM/provenance. Each architecture has a separate
`mode=max` layer cache. BuildKit cache mounts help local rebuilds, but their
contents are not exported by the GitHub layer cache. The web app, Rust binary,
FFmpeg, speaker model and ONNX Runtime come from independent stages: a UI-only
change rebuilds neither the binary nor the model download.

```sh
docker build -t fundus:local .
python3 scripts/container-smoke.py fundus:local
# Also verify proxy authentication without initializing an owner password:
python3 scripts/container-smoke.py fundus:local --proxy
```

The smoke check creates and removes disposable containers, an internal network
and a volume. A Python fixture image runs the provider mock and HTTP client.
Synthetic audio exercises startup rejection without a key, web assets,
tus offsets, real FFmpeg/ONNX inference, speaker enrollment, hybrid search, audio
Range, health checks and persistence after restart. No host recordings or API
keys are mounted, and runtime tests have no external network access.

## Deploy a published image

After the first GitHub publication, copy `compose.image.yaml` and `.env.example`
to the server. Save the latter as `.env`, fill in its settings and add:

```dotenv
VAULT_IMAGE=ghcr.io/OWNER/REPOSITORY@sha256:PUBLISHED_MANIFEST_DIGEST
```

```sh
# Authenticate to GHCR first if the package is private.
docker compose -f compose.image.yaml pull
python3 -c 'import getpass; print(getpass.getpass("Vault password: "))' | docker compose -f compose.image.yaml run --rm -T server init
docker compose -f compose.image.yaml up -d
```

Skip `init` for an existing installation. Keep the Compose directory and project
name unchanged when switching from `compose.yaml`: both files use that project's
`vault-data` volume. Back up before upgrading. Change `VAULT_IMAGE`, pull and run
`up -d`; no host build tools are needed. Schema changes may require restoring the
backup before rolling back to an older image.

Both Compose files use non-root read-only containers, writable `/data`, temporary
scratch space, dropped capabilities, bounded logs and loopback HTTP. HTTPS still
belongs to the external reverse proxy. `compose.yaml` remains the source-build option.

## Mac bundles

`scripts/package-macos.sh` produces versioned ZIP, DMG and SHA-256 files in `build/`.
The `-local` suffix identifies test bundles: signed with a local
Developer ID or Apple Development identity when the keychain has one, ad hoc
otherwise (as on CI). CI uploads these as
`macos-local-bundle`. Version comes from `crates/server/Cargo.toml`, architecture
from the build host, and `VAULT_BUILD_NUMBER` sets the numeric build (default `1`).

For distribution, explicitly supply your own credentials:

```sh
MACOS_SIGNING_IDENTITY='Developer ID Application: YOUR NAME (TEAMID)' \
NOTARY_PROFILE=your-notary-profile VAULT_BUILD_NUMBER=2 \
scripts/package-macos.sh --release
```

Release mode requires a Developer ID identity, signs with the hardened runtime and a
secure timestamp, and never silently substitutes an ad hoc release. With
`NOTARY_PROFILE` it also checks Apple's acceptance and staples the app and DMG.
Without it the bundle runs on the Mac that built it, but Gatekeeper blocks it on
other Macs. Output names omit `-local`. Neither mode publishes artifacts.

## Mobile distribution and validation

Expo remains the shared setup with SwiftUI/Compose UI and native transfer modules.
The verified Android debug APK and unsigned iOS simulator app are development
outputs. Release APK/AAB signing and iOS provisioning/TestFlight need this app's
own signing configuration and device acceptance checks. Another app's identities,
keystores or provisioning profiles are not reused.

The isolated container/restart test passed on Linux AMD64 and ARM64 with the read-only
runtime restrictions. Mac ZIP/DMG creation, checksums and DMG verification passed.
Hosted AMD64/ARM64 builds, registry publication and vulnerability scanning need
the first GitHub workflow run. Notarization was not executed.
Keep installation-specific domains, credentials, server paths and operational
runbooks outside this repository.
