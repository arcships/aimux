plugins {
    kotlin("jvm") version "2.0.0"
    `java-library`
    `maven-publish`
    signing
    id("org.jetbrains.dokka") version "1.9.20"
}

group = "ai.arcships"
version = "0.5.0"

repositories {
    mavenCentral()
}

dependencies {
    api("ai.arcships:aimux-java:$version") {
        version { strictly(project.version.toString()) }
    }
    api("org.jetbrains.kotlinx:kotlinx-coroutines-core:1.7.3")
    // Java's decoded error payload types are part of the Kotlin public facade.
    api("com.fasterxml.jackson.core:jackson-databind:2.17.2")
    testImplementation("org.junit.jupiter:junit-jupiter:5.10.0")
}

val verifyJavaArtifact by tasks.registering {
    doLast {
        val javaArtifact = configurations["compileClasspath"].incoming.resolutionResult
            .allComponents.mapNotNull { it.moduleVersion }
            .single { it.group == "ai.arcships" && it.name == "aimux-java" }
        check(javaArtifact.version == project.version.toString()) {
            "Kotlin ${project.version} requires the same Java artifact version, got $javaArtifact"
        }
    }
}

tasks.named("compileKotlin") {
    dependsOn(verifyJavaArtifact)
}

// Keep published bytecode and tests on JVM 17.
kotlin {
    jvmToolchain(17)
}

tasks.test {
    useJUnitPlatform()
    systemProperty(
        "jna.library.path",
        System.getenv("AIMUX_FFI_LIB_DIR") ?: projectDir.resolve("../../target/debug").canonicalPath
    )
    System.getenv("AIMUX_LIBRARY_PATH")?.let { systemProperty("aimux.library", it) }
    testLogging {
        events("failed")
        exceptionFormat = org.gradle.api.tasks.testing.logging.TestExceptionFormat.FULL
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

// Publishing credentials come from Gradle properties; absent keys leave signing inert.
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
