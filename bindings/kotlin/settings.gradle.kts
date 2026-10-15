rootProject.name = "aimux-kotlin"

// Resolve the published coordinate against the matching source artifact in this checkout.
// The dependency in the POM remains ai.arcships:aimux-java:<same version>.
includeBuild("../java") {
    dependencySubstitution {
        substitute(module("ai.arcships:aimux-java")).using(project(":"))
    }
}
