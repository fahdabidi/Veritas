plugins {
    id("com.android.application")
    id("org.jetbrains.kotlin.android")
}

android {
    namespace = "com.veritas.gbn.mobile"
    compileSdk = 36

    defaultConfig {
        applicationId = "com.veritas.gbn.mobile"
        minSdk = 26
        targetSdk = 36
        versionCode = 1
        versionName = "0.1.0-pass4-phase5"
        testInstrumentationRunner = "androidx.test.runner.AndroidJUnitRunner"
    }

    buildFeatures {
        buildConfig = true
    }

    buildTypes {
        debug {
            applicationIdSuffix = ".debug"
            versionNameSuffix = "-debug"
        }
        release {
            isMinifyEnabled = false
        }
    }

    compileOptions {
        sourceCompatibility = JavaVersion.VERSION_17
        targetCompatibility = JavaVersion.VERSION_17
    }
}

kotlin {
    compilerOptions {
        jvmTarget.set(org.jetbrains.kotlin.gradle.dsl.JvmTarget.JVM_17)
    }
}

dependencies {
    testImplementation("junit:junit:4.12")
    androidTestImplementation("junit:junit:4.12")
    androidTestImplementation("androidx.test:runner:1.2.0")
    androidTestImplementation("androidx.test:rules:1.2.0")
    androidTestImplementation("androidx.test:monitor:1.2.0")
}

val protoRoot = rootProject.projectDir.parentFile.parentFile

tasks.register<Exec>("buildMobileFfiDebug") {
    workingDir = protoRoot
    commandLine(
        "bash",
        "infra/scripts/build-mobile-ffi.sh",
        "--abi",
        "arm64-v8a",
        "--abi",
        "x86_64",
        "--profile",
        "debug",
    )
}

tasks.named("preBuild") {
    dependsOn("buildMobileFfiDebug")
}
