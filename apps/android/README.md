# Fundus for Android

Native Kotlin and Jetpack Compose (Material 3). Connects to a vault through browser approval, then
lists recordings (search, pull to refresh, swipe to trash with undo), shows transcripts with
playback, and names speakers. `apps/mobile` (Expo) stays the upload client until this app uploads.

## Build

Needs the Android SDK (platform 37) and a JDK that Gradle supports; Android Studio's bundled one works:

```sh
export JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"
./gradlew testDebugUnitTest        # the OAuth, decoding and formatting checks
./gradlew assembleRelease          # app/build/outputs/apk/release/app-release.apk, about 4 MB
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
- Colours follow `DESIGN.md` (white, neutral ink, red only for problems), mapped onto Material 3 roles.
