package org.nanobug.maven;

import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.nio.file.StandardCopyOption;
import java.util.Arrays;
import org.apache.maven.AbstractMavenLifecycleParticipant;
import org.apache.maven.MavenExecutionException;
import org.apache.maven.execution.MavenSession;

/** Install only this immutable helper into the repository selected by Maven's effective settings. */
public final class Bootstrap extends AbstractMavenLifecycleParticipant {
    /** Session creation has resolved the local repository; no user model or settings parser is copied. */
    @Override public void afterSessionStart(MavenSession session) throws MavenExecutionException {
        try {
            Path destination = session.getRepositorySession().getLocalRepository().getBasedir().toPath()
                .resolve("org/nanobug/plugins/maven-model/" + Bridge.VERSION);
            Files.createDirectories(destination);
            Path jar = Paths.get(Bootstrap.class.getProtectionDomain().getCodeSource().getLocation().toURI());
            put(destination.resolve("maven-model-" + Bridge.VERSION + ".jar"), Files.readAllBytes(jar));
            try (InputStream input = Bootstrap.class.getResourceAsStream("/META-INF/maven/org.nanobug.plugins/maven-model/pom.xml")) {
                if (input == null) throw new IllegalStateException("Missing helper POM");
                ByteArrayOutputStream bytes = new ByteArrayOutputStream();
                byte[] block = new byte[4096];
                for (int count; (count = input.read(block)) >= 0;) bytes.write(block, 0, count);
                put(destination.resolve("maven-model-" + Bridge.VERSION + ".pom"), bytes.toByteArray());
            }
        } catch (Exception error) { throw new MavenExecutionException("Cannot prepare the immutable Nanobug import goal", error); }
    }
    /** Never replace a conflicting helper at the same version; concurrent equal publication is harmless. */
    private static void put(Path path, byte[] bytes) throws Exception {
        if (Files.exists(path)) {
            if (!Arrays.equals(Files.readAllBytes(path), bytes)) throw new IllegalStateException("Conflicting Maven helper identity");
            return;
        }
        Path temporary = Files.createTempFile(path.getParent(), ".nanobug-", ".tmp");
        try {
            Files.write(temporary, bytes);
            try { Files.move(temporary, path, StandardCopyOption.ATOMIC_MOVE); }
            catch (java.nio.file.FileAlreadyExistsException error) {
                if (!Arrays.equals(Files.readAllBytes(path), bytes)) throw error;
            }
        } finally { Files.deleteIfExists(temporary); }
    }
}
