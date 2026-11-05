# Fundus for Android

Native Kotlin and Jetpack Compose (Material 3). Connects to a vault through browser approval, then
lists recordings (search, pull to refresh, swipe to trash with undo), shows transcripts with
playback, and names speakers. Plugging in a DJI Mic opens the same window as on the Mac: its
recordings, newest first, each with its own Import button.

## Build

Needs the Android SDK (platform 37), the NDK version pinned as `ndkVersion` in `app/build.gradle.kts`
and CMake 3.22.1 (SDK Manager), and a JDK that Gradle supports; Android Studio's bundled one works.
CI runs the same unit tests (`.github/workflows/check.yml`):

```sh
../../scripts/fetch-native-deps.sh     # libFLAC source, once
export JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"
./gradlew testDebugUnitTest        # OAuth, decoding, session grouping, import status, formatting
./gradlew assembleRelease          # app/build/outputs/apk/release/app-release.apk, about 5 MB
adb install -r app/build/outputs/apk/release/app-release.apk
```

Release builds are signed with the debug key so they install over debug builds; give the app its
own keystore before handing the APK to anyone else.

## How it fits

- The models, API calls and OAuth follow the Mac app (`apps/macos/Sources/Fundus`): same endpoints
  under `/api/native/v1`, same wording, same rules for naming speakers.
- The vault's approval page returns to `dev.wsky.fundus:/oauth`; the server accepts that redirect.
  Tokens are stored encrypted with a key held in the Android Keystore.
- HTTP is allowed only for `127.0.0.1` and `localhost`, for a development vault reached with
  `adb reverse tcp:3917 tcp:3917`.
- Uploads follow the Mac too: sessions grouped by the same rules (30-minute rollovers join), the
  same source IDs, so either app sees what the other imported through the vault's source history.
  Recordings are copied off the mic first, so it can be unplugged; integer WAV up to 24 bits becomes
  verified FLAC through `native/audio` (float and 32-bit go up unchanged, as on the Mac); tus chunks
  run in a foreground service. An upload the app was closed in the middle of resumes when it starts
  again; a paused or failed one waits for Resume in the window, with a notification unless Android's
  limit on background transfers stopped it.
- The DJI case is a USB drive that Android hides from apps (no storage volume, no `MEDIA_MOUNTED`).
  Its USB device (vendor `0x10D6`, `res/xml/dji_mic.xml`) starts the app instead; the files are read
  through the system folder picker, picked once per mic ("Allow" in the window). Header reads are
  cached across launches.
- DJI WAVs declare 8 more data bytes than they hold; like Core Audio on the Mac, only whole frames
  actually in the file count, so frame counts and source IDs match.
- The recorder clock (DJI factory UTC+8, or the phone's zone) is taken once from the vault's newest
  recording, then kept until another one is picked in the window's footer.
- Colours follow `DESIGN.md` (white, neutral ink, red only for problems), mapped onto Material 3 roles.
