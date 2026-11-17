// A native activity without Java code: Gradle only builds libfreakymate.so with cargo-ndk, packs it with the manifest
// and signs the APK.
//   ./gradlew assembleDevRelease -PappVersion=0.6.1-dev   (appVersion as .github/scripts/version.sh prints it)
//   ./gradlew assembleProdRelease -PappVersion=0.6.3      (CI on main only, needs the release key)
// -Pabis=x86_64 builds for the emulator (`rustup target add x86_64-linux-android`); the release is arm64 only.
// Release key: ANDROID_KEYSTORE (file), ANDROID_KEYSTORE_PASSWORD, alias `freakymate`.
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
val releaseKeystore: String? = System.getenv("ANDROID_KEYSTORE")
val androidHome: String = System.getenv("ANDROID_HOME") ?: error("set ANDROID_HOME")
val jniLibs = layout.buildDirectory.dir("jniLibs")
val cargoNdk = tasks.register<Exec>("cargoNdk") {
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
        releaseKeystore?.let { keystore ->
            create("release") {
                storeFile = file(keystore)
                storePassword = System.getenv("ANDROID_KEYSTORE_PASSWORD")
                keyAlias = "freakymate"
                keyPassword = System.getenv("ANDROID_KEYSTORE_PASSWORD")
            }
        }
    }

    // prod is FreakyMate, built only from main and always with the release key; dev is every other build (local, pull
    // request): its own app beside it with its own data, signed with the debug key of the machine.
    flavorDimensions += "identity"
    productFlavors {
        create("prod") {
            dimension = "identity"
            manifestPlaceholders["appLabel"] = "FreakyMate"
            signingConfig = signingConfigs.findByName("release")
        }
        create("dev") {
            dimension = "identity"
            applicationIdSuffix = ".dev"
            manifestPlaceholders["appLabel"] = "FreakyMate Dev"
            signingConfig = signingConfigs.getByName("debug")
        }
    }
}

tasks.named("preBuild") { dependsOn(cargoNdk) }
// Without the key Gradle would pack an unsigned or debug-signed prod APK that no later release could update.
tasks.matching { it.name == "preProdReleaseBuild" }.configureEach {
    doFirst { check(releaseKeystore != null) { "prod needs the release key: set ANDROID_KEYSTORE and ANDROID_KEYSTORE_PASSWORD" } }
}
