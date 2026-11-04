plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.plugin.compose")
    id("org.jetbrains.kotlin.plugin.serialization")
}

android {
    namespace = "dev.wsky.fundus"
    compileSdk = 37

    defaultConfig {
        applicationId = "dev.wsky.fundus"
        minSdk = 29
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0"
    }

    buildTypes {
        release {
            isMinifyEnabled = true
            isShrinkResources = true
            proguardFiles(getDefaultProguardFile("proguard-android-optimize.txt"))
            // ponytail: signed with the debug key so it installs over debug builds; add a release
            // keystore before handing the APK to anyone else.
            signingConfig = signingConfigs.getByName("debug")
        }
    }

    buildFeatures {
        compose = true
    }
}

dependencies {
    implementation(platform("androidx.compose:compose-bom:2026.09.00"))
    // Material 3 Expressive (motion, loading indicator, wavy progress) is complete only in 1.5.
    implementation("androidx.compose.material3:material3:1.5.0-beta02")
    implementation("androidx.compose.material:material-icons-extended:1.7.8")
    implementation("androidx.core:core-ktx:1.19.1")
    implementation("androidx.activity:activity-compose:1.13.0")
    implementation("androidx.navigation:navigation-compose:2.10.2")
    implementation("androidx.lifecycle:lifecycle-viewmodel-compose:2.11.0")
    implementation("androidx.lifecycle:lifecycle-runtime-compose:2.11.0")
    implementation("androidx.browser:browser:1.10.0")
    implementation("androidx.media3:media3-exoplayer:1.11.1")
    implementation("androidx.media3:media3-datasource-okhttp:1.11.1")
    implementation("androidx.media3:media3-session:1.11.1")
    implementation("com.squareup.okhttp3:okhttp:5.5.1")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.12.0")

    testImplementation("junit:junit:4.13.2")
}
