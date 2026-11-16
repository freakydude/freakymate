// A native activity without Java code: Gradle only builds libfreakymate.so with cargo-ndk, packs it with the manifest
// and signs the APK.
//   ./gradlew assembleRelease -PappVersion=0.6.1      (appVersion as .github/scripts/version.sh prints it)
// -Pabis=x86_64 builds for the emulator (`rustup target add x86_64-linux-android`); the release is arm64 only.
// Signing: ANDROID_KEYSTORE (file), ANDROID_KEYSTORE_PASSWORD, alias `freakymate`. Without them the APK gets the debug
// key of the machine; it must be uninstalled before a release APK can be installed.
plugins {
    id("com.android.application")
}

val compileSdkLevel = 36
val appVersion = providers.gradleProperty("appVersion").orElse("0.0.1-dev").get()

// X.Y.Z -> X*10_000_000 + Y*10_000 + Z (suffixes such as -pr.1.2 are ignored); stays below Android's limit of 2.1 billion.
fun versionCodeOf(version: String): Int {
    val (major, minor, patch) = version.substringBefore('-').split('.').map { it.toInt() }
    return major * 10_000_000 + minor * 10_000 + patch
}

val abis = providers.gradleProperty("abis").orElse("arm64-v8a").get().split(",")
val androidHome: String = System.getenv("ANDROID_HOME") ?: error("set ANDROID_HOME")
val jniLibs = layout.buildDirectory.dir("jniLibs")
val cargoNdk by tasks.registering(Exec::class) {
    workingDir = rootDir.parentFile
    // slint's build script does not find the platform on its own.
    environment("ANDROID_JAR", "$androidHome/platforms/android-$compileSdkLevel/android.jar")
    commandLine(
        listOf("cargo", "ndk") + abis.flatMap { listOf("-t", it) } +
            listOf("-o", jniLibs.get().asFile.absolutePath, "build", "--release", "-p", "freakymate-android"),
    )
}

base.archivesName.set("freakymate-$appVersion-${abis.joinToString("+")}")

android {
    namespace = "de.freakydu.freakymate"
    compileSdk = compileSdkLevel
    // Needed to strip the library; the same NDK as .github/scripts/setup-android.sh installs.
    ndkVersion = "30.0.16248370"

    defaultConfig {
        applicationId = "de.freakydu.freakymate"
        minSdk = 24
        targetSdk = compileSdkLevel
        versionCode = versionCodeOf(appVersion)
        versionName = appVersion
        ndk { abiFilters += abis }
    }

    sourceSets["main"].jniLibs.directories.add(jniLibs.get().asFile.absolutePath)

    // Compressed in the APK: 15 instead of 41 MB to download; the library is unpacked on install.
    packaging.jniLibs.useLegacyPackaging = true

    signingConfigs {
        System.getenv("ANDROID_KEYSTORE")?.let { keystore ->
            create("release") {
                storeFile = file(keystore)
                storePassword = System.getenv("ANDROID_KEYSTORE_PASSWORD")
                keyAlias = "freakymate"
                keyPassword = System.getenv("ANDROID_KEYSTORE_PASSWORD")
            }
        }
    }

    buildTypes {
        release {
            signingConfig = signingConfigs.findByName("release") ?: signingConfigs.getByName("debug")
        }
    }
}

tasks.named("preBuild") { dependsOn(cargoNdk) }
