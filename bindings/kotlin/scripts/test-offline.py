#!/usr/bin/env python3
"""Compile and run bridge tests from cached jars when Gradle cannot bind its lock socket."""
from pathlib import Path
import os
import shutil
import subprocess
import textwrap

binding = Path(__file__).resolve().parents[1]
cache = Path.home() / '.gradle/caches/modules-2/files-2.1'
out = Path(os.environ.get('AIMUX_KOTLIN_CHECK_DIR', binding / 'build/offline-check'))
java_home = os.environ.get('JAVA_HOME')
if not java_home:
    raise SystemExit('JAVA_HOME is required; run through mise exec java@temurin-17 gradle@8')
java_classes = out / 'java'
kotlin_classes = out / 'kotlin'
for classes in (java_classes, kotlin_classes):
    if classes.exists():
        shutil.rmtree(classes)
java_classes.mkdir(parents=True, exist_ok=True)
kotlin_classes.mkdir(parents=True, exist_ok=True)


def jar(group, name, version):
    matches = list((cache / group / name / version).glob(f'*/{name}-{version}.jar'))
    if len(matches) != 1:
        raise SystemExit(f'Expected one cached {group}:{name}:{version}, found {len(matches)}')
    return str(matches[0])


def run(*args):
    executable = Path(java_home) / 'bin' / args[0]
    print('Running:', executable, flush=True)
    timeout = int(os.environ.get('AIMUX_KOTLIN_COMMAND_TIMEOUT', '180'))
    subprocess.run([str(executable), *map(str, args[1:])], check=True, timeout=timeout)


runtime = [
    jar('org.jetbrains.kotlin', 'kotlin-stdlib', '2.0.0'),
    jar('org.jetbrains.kotlinx', 'kotlinx-coroutines-core-jvm', '1.7.3'),
    jar('net.java.dev.jna', 'jna', '5.14.0'),
    *[jar('com.fasterxml.jackson.core', name, '2.17.2') for name in
      ('jackson-annotations', 'jackson-core', 'jackson-databind')],
    jar('org.junit.jupiter', 'junit-jupiter-api', '5.10.0'),
    jar('org.junit.platform', 'junit-platform-commons', '1.10.0'),
    jar('org.opentest4j', 'opentest4j', '1.3.0'),
    jar('org.apiguardian', 'apiguardian-api', '1.1.2'),
    jar('org.jetbrains', 'annotations', '13.0'),
]
compiler = [
    jar('org.jetbrains.kotlin', 'kotlin-compiler-embeddable', '2.0.0'),
    jar('org.jetbrains.kotlin', 'kotlin-script-runtime', '2.0.0'),
    jar('org.jetbrains.kotlin', 'kotlin-reflect', '1.6.10'),
    jar('org.jetbrains.intellij.deps', 'trove4j', '1.0.20200330'),
    *runtime,
]
cp = os.pathsep.join(runtime)
existing_jar = os.environ.get('AIMUX_JAVA_TEST_JAR')
if existing_jar:
    java_jar = Path(existing_jar).resolve(strict=True)
else:
    sources = sorted((binding.parent / 'java/src/main/java').rglob('*.java'))
    sources += sorted((binding.parent / 'java/generated').rglob('*.java'))
    run('javac', '--release', '8', '-encoding', 'UTF-8', '-cp', cp, '-d', java_classes, *sources)
    java_jar = out / 'aimux-java.jar'
    run('jar', 'cf', java_jar, '-C', java_classes, '.')
cp = os.pathsep.join([str(java_jar), *runtime])
fixtures = sorted((binding / 'src/test/java').rglob('*.java'))
run('javac', '--release', '8', '-encoding', 'UTF-8', '-cp', cp, '-d', kotlin_classes, *fixtures)
sources = sorted((binding / 'src/main/kotlin').rglob('*.kt'))
sources += sorted((binding / 'src/test/kotlin').rglob('*.kt'))
cp = os.pathsep.join([str(kotlin_classes), cp])
run('java', '-cp', os.pathsep.join(compiler), 'org.jetbrains.kotlin.cli.jvm.K2JVMCompiler',
    '-no-stdlib', '-no-reflect', '-jvm-target', '17', '-classpath', cp,
    '-d', kotlin_classes, *sources)

# Run the actual @Test methods with JUnit assertions.
runner = out / 'OfflineTestRunner.java'
runner.write_text(textwrap.dedent('''\
    import java.lang.reflect.*;
    import java.util.*;
    import org.junit.jupiter.api.Test;
    public final class OfflineTestRunner {
        public static void main(String[] args) throws Exception {
            int passed = 0;
            int failed = 0;
            for (String name : args) {
                Class<?> type = Class.forName(name);
                Method[] methods = type.getDeclaredMethods();
                Arrays.sort(methods, Comparator.comparing(Method::getName));
                for (Method method : methods) {
                    if (!method.isAnnotationPresent(Test.class)) continue;
                    try {
                        method.invoke(type.getDeclaredConstructor().newInstance());
                        System.out.println("PASS " + type.getSimpleName() + "." + method.getName());
                        passed++;
                    } catch (InvocationTargetException error) {
                        failed++;
                        System.err.println("FAIL " + type.getSimpleName() + "." + method.getName());
                        error.getCause().printStackTrace();
                    }
                }
            }
            System.out.println("Tests: " + passed + " passed, " + failed + " failed");
            if (failed != 0 || passed == 0) System.exit(1);
        }
    }
'''))
run('javac', '-cp', cp, '-d', kotlin_classes, runner)
tests = ['ai.arcships.aimux.kotlin.ErrorsTest', 'ai.arcships.aimux.kotlin.CoroutinesTest',
         'ai.arcships.aimux.internal.FlowBridgeTest']
library = os.environ.get('AIMUX_LIBRARY_PATH')
if library:
    tests.append('ai.arcships.aimux.kotlin.NativeIntegrationTest')
else:
    print('NativeIntegrationTest not run: set AIMUX_LIBRARY_PATH to the built cdylib.', flush=True)
run('java', *(['-Daimux.library=' + library] if library else []), '-cp', cp,
    'OfflineTestRunner', *tests)
