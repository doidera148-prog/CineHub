plugins {
    id("com.android.library")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.cinehub.exoplayer"
    compileSdk = 36

    defaultConfig {
        minSdk = 24
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_1_8
        targetCompatibility = JavaVersion.VERSION_1_8
    }
    kotlinOptions {
        jvmTarget = "1.8"
    }
}

// Extensão FFmpeg do Media3 (áudio que o aparelho não decodifica). Não está no Maven: é compilada
// pelo workflow "ffmpeg-extension" e publicada como release "ffmpeg-ext-1.4.1"; o job Android a baixa
// para libs/. Sem o arquivo, o app compila normalmente e usa só os decodificadores do aparelho.
val ffmpegExtension = file("libs/media3-decoder-ffmpeg-1.4.1.aar")
if (ffmpegExtension.exists()) {
    repositories {
        flatDir { dirs("libs") }
    }
}

dependencies {
    if (ffmpegExtension.exists()) {
        implementation(name = "media3-decoder-ffmpeg-1.4.1", ext = "aar")
    }
    api(project(":tauri-android"))
    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.media3:media3-exoplayer:1.4.1")
    implementation("androidx.media3:media3-exoplayer-hls:1.4.1")
    implementation("androidx.media3:media3-exoplayer-dash:1.4.1")
    implementation("androidx.media3:media3-ui:1.4.1")
}
