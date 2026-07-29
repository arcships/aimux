plugins {
    kotlin("jvm") version "2.0.0"
    kotlin("plugin.serialization") version "2.0.0"
}

group = "io.aimux"
version = "0.1.0"

repositories {
    mavenCentral()
}

dependencies {
    implementation("net.java.dev.jna:jna:5.14.0")
    implementation("org.jetbrains.kotlinx:kotlinx-serialization-json:1.7.0")
    testImplementation("org.junit.jupiter:junit-jupiter:5.10.0")
    testImplementation("org.assertj:assertj-core:3.26.0")
    testImplementation("org.json:json:20240303")
}

tasks.test {
    useJUnitPlatform()
}

// Package the native library into the JAR for distribution.
// In production, use per-platform JARs (like napi-rs does for Node).
tasks.jar {
    from("src/main/resources") {
        into("native")
    }
}
