package org.nanobug.maven;

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.util.ArrayList;
import java.util.List;
import org.apache.maven.cli.MavenCli;

/** An owned JVM runs one metadata-only Maven goal, with model output separate from ordinary logs. */
public final class Bridge {
    public static final String VERSION = "0.1.18";
    /** Args are checked workspace/POM/private data plus repository, profile and offline choices. */
    public static void main(String[] args) throws Exception {
        if (args.length < 7) throw new IllegalArgumentException("Invalid Maven bridge request");
        Path workspace = Paths.get(args[0]).toRealPath();
        Path pom = Paths.get(args[1]).toRealPath();
        if (!pom.startsWith(workspace)) throw new IllegalArgumentException("POM escapes workspace");
        Path data = Paths.get(args[2]);
        Path directory = Paths.get(args[6]).toRealPath();
        boolean goal = args.length > 7 && "goal".equals(args[7]);
        Path report = data.resolve("model.json");
        if (!goal) {
            Files.createDirectories(data);
            Files.deleteIfExists(report);
            System.setProperty("nanobug.maven.report", report.toString());
        }
        System.setProperty("nanobug.maven.workspace", workspace.toString());
        List<String> cli = new ArrayList<>();
        java.util.Collections.addAll(cli, "-B", "-ntp", "-Dstyle.color=never", "-f", pom.toString());
        if (!args[3].isEmpty()) cli.add("-Dmaven.repo.local=" + args[3]);
        if (!args[4].isEmpty()) cli.add("-P" + args[4]);
        if (Boolean.parseBoolean(args[5])) cli.add("-o");
        if (goal) {
            if (args.length < 9) throw new IllegalArgumentException("Maven requires a goal");
            java.util.Collections.addAll(cli, java.util.Arrays.copyOfRange(args, 8, args.length));
        } else cli.add("org.nanobug.plugins:maven-model:" + VERSION + ":import");
        int code;
        try {
            code = new MavenCli().doMain(cli.toArray(new String[0]), directory.toString(), goal ? System.out : System.err, System.err);
            if (code == 0 && !goal) {
                if (!Files.isRegularFile(report) || Files.size(report) > 48 * 1024)
                    throw new IllegalStateException("Missing or oversized Maven model");
                System.out.write(Files.readAllBytes(report));
                System.out.flush();
            }
        } finally {
            // Only this import's private report directory is owned here; repository artifacts stay.
            if (!goal) { Files.deleteIfExists(report); Files.deleteIfExists(data); }
        }
        if (code != 0) System.exit(code);
    }
}
