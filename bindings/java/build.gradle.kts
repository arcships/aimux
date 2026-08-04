plugins {
    `java-library`
    `maven-publish`
    signing
}

group = "ai.arcships"
version = "0.2.1"

repositories {
    mavenCentral()
}

java {
    sourceCompatibility = JavaVersion.VERSION_1_8
    targetCompatibility = JavaVersion.VERSION_1_8
}

tasks.withType<JavaCompile>().configureEach {
    options.encoding = "UTF-8"
    // JDK 8's javac has no --release flag; only set it on JDK 9+ so the
    // library stays compilable with a JDK 8 toolchain (used by the CI matrix).
    if (JavaVersion.current().isJava9Compatible) {
        options.release.set(8)
    }
    // Compiler enforces Java 8 syntax: no var/record/switch expressions, etc.
}

dependencies {
    implementation("net.java.dev.jna:jna:5.14.0")
    implementation("com.fasterxml.jackson.core:jackson-databind:2.17.2")
    testImplementation("org.junit.jupiter:junit-jupiter:5.10.0")
    testImplementation("org.assertj:assertj-core:3.26.0")
    testImplementation("org.json:json:20240303")
}

tasks.test {
    useJUnitPlatform()
    testLogging {
        events("passed", "skipped", "failed")
    }
}

// Javadoc for a Java 8 bytecode target can trip doclint (e.g. on JDK 9+);
// disable it so `publishToMavenLocal` / `publish` work on any toolchain.
tasks.withType<Javadoc>().configureEach {
    (options as StandardJavadocDocletOptions).addStringOption("Xdoclint:none", "-quiet")
}

val sourcesJar by tasks.registering(Jar::class) {
    archiveClassifier.set("sources")
    from(sourceSets["main"].allSource)
}

val javadocJar by tasks.registering(Jar::class) {
    archiveClassifier.set("javadoc")
    from(tasks.named("javadoc"))
}

// ─────────────────────────────────────────────────────────────────────────────
// Maven Central (Sonatype OSSRH) publishing.
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
                name.set("aimux-java")
                description.set("Java binding for aimux — a Rust alternative to the Vercel AI SDK: a unified provider interface for LLM applications")
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
            // Central Publisher Portal — OSSRH Staging API compatibility
            // service (s01.oss.sonatype.org is deprecated; it returns 402).
            // After uploading, release.yml POSTs /manual/upload/defaultRepository
            // to move the deployment into the Portal.
            // Snapshot versions go to the snapshots repository; release
            // versions to the staging service (the snapshots repo rejects
            // non-SNAPSHOT versions with 403).
            if (version.toString().endsWith("SNAPSHOT")) {
                maven {
                    name = "CentralPortal"
                    url = uri("https://central.sonatype.com/repository/maven-snapshots/")
                    credentials {
                        username = ossrhUsername
                        password = ossrhPassword
                    }
                }
            } else {
                maven {
                    name = "CentralPortal"
                    url = uri("https://ossrh-staging-api.central.sonatype.com/service/local/staging/deploy/maven2/")
                    credentials {
                        username = ossrhUsername
                        password = ossrhPassword
                    }
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
