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
// pelo workflow "ffmpeg-extension" e publicada no release "ffmpeg-ext-1.4.1". O job Android baixa o AAR
// e o desempacota aqui: classes.jar em libs/ e as bibliotecas nativas em src/main/jniLibs/.
// Sem esses arquivos, o app compila normalmente e usa só os decodificadores do aparelho.
val ffmpegClasses = file("libs/media3-decoder-ffmpeg-1.4.1-classes.jar")

dependencies {
    if (ffmpegClasses.exists()) {
        implementation(files(ffmpegClasses))
    }
    api(project(":tauri-android"))
    implementation("androidx.core:core-ktx:1.13.1")
    implementation("androidx.media3:media3-exoplayer:1.4.1")
    implementation("androidx.media3:media3-exoplayer-hls:1.4.1")
    implementation("androidx.media3:media3-exoplayer-dash:1.4.1")
    implementation("androidx.media3:media3-ui:1.4.1")
}
