# Maven for Nanobug

[简体中文](README.zh-CN.md)

Independent Maven project import and build integration for Nanobug. Gradle, Lombok and JUnit remain
separate plugins. This local package has not been published to a registry.

Build with `Nanobug.exe --plugin-package C:/Projects/RustProjects/Maven-for-Nanobug`.
The host exports the public SDK; no host checkout or SDK copy is embedded in the guest project.
The Windows x86-64 build uses Cargo, `javac`, `jar` and `tar` directly. Runtime requires protocol 7
and `language.lsp` 1.9. Java analysis, application launch and debugging additionally use a Java plugin
providing `java.project` 1.2; Maven never implements those features itself.

Install the ZIP through Nanobug's plugin manager and approve its declared permissions. Trust the
workspace before starting tools. Open the **Maven Project** panel, choose a workspace-relative POM,
enter comma-separated profiles, and press Import / Refresh. Import evaluates the real effective
reactor and resolves dependencies without compiling or launching application code. Choose
`maven/analysis` as the Java language provider. Refresh after changing build inputs; POM and `.mvn`
events withdraw previous contexts. Failed or cancelled imports retain a visibly stale descriptive
model, which cannot silently drive analysis or launches.

The native run configuration catalog includes **Maven Goal** and **Maven Java Application**. A goal
uses literal argument entries, environment and working directory; its output, stop and rerun use
ordinary native sessions. An application selects an imported module and main class, runs Maven's
reactor `compile` first, then delegates the resolved runtime classpath and JDK to Java. Debugging
uses Java's existing breakpoint, thread, stack and variable UI.

Settings `maven.home` and `maven.java` select explicit local tools; invalid overrides fail instead of
silently changing tools. Java 8 or newer must be on PATH or supplied with `maven.java` to run Maven.
Project setting `maven.repository` overrides the local repository; otherwise Maven owns settings,
mirrors and repository selection. Profiles and offline mode are available in the project panel and
run configurations. Standard `.mvn/maven.config` remains a Maven input.

The pinned Apache Maven 3.10.0 ZIP is verified against its publisher's checksum. The host extracts it
to a private dependency directory after installation consent. No global PATH is changed.
Apache distribution LICENSE and NOTICE remain inside `vendor/maven.zip`.

Analysis uses one shared Java session and separate main/test projects for each reactor module.
Only resolved visible dependencies enter each project, preserving mediation, exclusions and order.
Explicit compiler release/source/target and the standard `default-compile` configuration are exported
with Maven's expression evaluator. Unspecified compiler parameters use Java's configured defaults;
arbitrary compiler plugin defaults, toolchain selection and custom lifecycle executions are not
inferred. Maven Wrapper, Maven 4 and JVM arguments in `.mvn/jvm.config` are not claimed supported.
Import is bounded to 48 KiB of model
metadata, 64 Java modules and 32 KiB of Java analysis graph; excess data produces an explicit error.
