plugins {
    kotlin("jvm") version "2.0.0"
    kotlin("plugin.serialization") version "2.0.0"
    `maven-publish`
    signing
    id("org.jetbrains.dokka") version "1.9.20"
}

group = "io.aimux"
version = "0.1.1"

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

val sourcesJar by tasks.registering(Jar::class) {
    archiveClassifier.set("sources")
    from(sourceSets["main"].allSource)
}

val javadocJar by tasks.registering(Jar::class) {
    archiveClassifier.set("javadoc")
    from(tasks.named("dokkaJavadoc"))
}

// ─────────────────────────────────────────────────────────────────────────────
// Maven Central (Sonatype OSSRH) publishing — same layout as aimux-java.
//
// Secrets are NEVER stored in the repo. They are read from Gradle project
// properties — set either as `ORG_GRADLE_PROJECT_<name>` environment variables
// or `-P<name>=...` system properties:
//   signingKey / signingPassword   ASCII-armored GPG private key + passphrase
//   ossrhUsername / ossrhPassword  Sonatype (s01.oss.sonatype.org) credentials
// When the keys are absent, signing/publishing configuration stays inert, so
// local builds and `gradle test` are unaffected.
// ─────────────────────────────────────────────────────────────────────────────

val ossrhUsername = findProperty("ossrhUsername") as String?
val ossrhPassword = findProperty("ossrhPassword") as String?
val signingKey = findProperty("signingKey") as String?
val signingPassword = findProperty("signingPassword") as String?

publishing {
    publications {
        create<MavenPublication>("mavenJava") {
            from(components["java"])
            artifact(tasks.named("sourcesJar"))
            artifact(tasks.named("javadocJar"))

            pom {
                name.set("aimux-kotlin")
                description.set("Kotlin binding for aimux — a Rust alternative to the Vercel AI SDK: a unified provider interface for LLM applications")
                url.set("https://github.com/arcships/aimux")
                licenses {
                    license {
                        name.set("MIT")
                        url.set("https://opensource.org/licenses/MIT")
                        distribution.set("repo")
                    }
                }
                scm {
                    connection.set("scm:git:git://github.com/arcships/aimux.git")
                    developerConnection.set("scm:git:ssh://git@github.com/arcships/aimux.git")
                    url.set("https://github.com/arcships/aimux")
                }
                developers {
                    developer {
                        name.set("aimux contributors")
                        id.set("aimux")
                        url.set("https://github.com/arcships/aimux")
                    }
                }
            }
        }
    }

    repositories {
        if (ossrhUsername != null && ossrhPassword != null) {
            maven {
                name = "OSSRH"
                url = uri("https://s01.oss.sonatype.org/service/local/staging/deploy/maven2/")
                credentials {
                    username = ossrhUsername
                    password = ossrhPassword
                }
            }
            maven {
                name = "OSSRHSnapshots"
                url = uri("https://s01.oss.sonatype.org/content/repositories/snapshots/")
                credentials {
                    username = ossrhUsername
                    password = ossrhPassword
                }
            }
        }
    }
}

signing {
    if (signingKey != null && signingPassword != null) {
        useInMemoryPgpKeys(signingKey, signingPassword)
        sign(publishing.publications["mavenJava"])
    }
}
