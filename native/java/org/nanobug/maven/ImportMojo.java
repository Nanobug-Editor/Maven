package org.nanobug.maven;

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.nio.file.Paths;
import java.security.MessageDigest;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.HashSet;
import java.util.Set;
import org.apache.maven.execution.MavenSession;
import org.apache.maven.plugin.AbstractMojo;
import org.apache.maven.plugin.MojoExecutionException;
import org.apache.maven.project.MavenProject;
import org.apache.maven.project.ProjectDependenciesResolver;
import org.apache.maven.project.DefaultDependencyResolutionRequest;
import org.eclipse.aether.artifact.Artifact;
import org.eclipse.aether.graph.Dependency;

/** A direct aggregator goal sees Maven's completed effective reactor without invoking compile phases. */
public final class ImportMojo extends AbstractMojo {
    /** Maven injects the actual session through the package's closed plugin descriptor. */
    private MavenSession session;
    /** The actual Resolver applies Maven's BOMs, exclusions, mediation and scope propagation. */
    private ProjectDependenciesResolver resolver;
    /** Export only domain facts; settings credentials and native process identities never enter the model. */
    @Override public void execute() throws MojoExecutionException {
        try {
            Path workspace = Paths.get(System.getProperty("nanobug.maven.workspace")).toRealPath();
            List<Object> modules = new ArrayList<>();
            Set<String> reactor = new HashSet<>();
            for (MavenProject project : session.getProjects())
                reactor.add(project.getGroupId() + ":" + project.getArtifactId() + ":" + project.getVersion());
            for (MavenProject project : session.getProjects()) {
                Map<String,Object> module = new LinkedHashMap<>();
                module.put("id", project.getGroupId() + ":" + project.getArtifactId() + ":" + project.getVersion());
                module.put("root", relative(workspace, project.getBasedir().toPath()));
                module.put("packaging", project.getPackaging());
                List<String> roots = new ArrayList<>();
                for (String root : project.getCompileSourceRoots()) roots.add(relative(workspace, Paths.get(root)));
                module.put("source_roots", roots);
                List<String> testRoots = new ArrayList<>();
                for (String root : project.getTestCompileSourceRoots()) testRoots.add(relative(workspace, Paths.get(root)));
                module.put("test_source_roots", testRoots);
                List<String> resources = new ArrayList<>();
                for (org.apache.maven.model.Resource resource : project.getResources())
                    resources.add(relative(workspace, Paths.get(resource.getDirectory())));
                module.put("resources", resources);
                List<String> testResources = new ArrayList<>();
                for (org.apache.maven.model.Resource resource : project.getTestResources())
                    testResources.add(relative(workspace, Paths.get(resource.getDirectory())));
                module.put("test_resources", testResources);
                module.put("output", relative(workspace, Paths.get(project.getBuild().getOutputDirectory())));
                module.put("test_output", relative(workspace, Paths.get(project.getBuild().getTestOutputDirectory())));
                Map<String,Object> compiler = compiler(project);
                module.put("compiler", compiler);
                String release = (String) compiler.get("release");
                module.put("jdk", !release.isEmpty() ? release : newer((String)compiler.get("source"),(String)compiler.get("target")));
                module.put("dependencies", dependencies(project, reactor));
                modules.add(module);
            }
            Map<String,Object> report = new LinkedHashMap<>();
            report.put("schema", 1);
            report.put("modules", modules);
            byte[] body = Json.write(report).getBytes(StandardCharsets.UTF_8);
            report.put("generation", hex(MessageDigest.getInstance("SHA-256").digest(body)));
            byte[] bytes = Json.write(report).getBytes(StandardCharsets.UTF_8);
            if (bytes.length > 48 * 1024) throw new IllegalStateException("Maven model response quota exceeded");
            Files.write(Paths.get(System.getProperty("nanobug.maven.report")), bytes);
        } catch (Exception error) { throw new MojoExecutionException("Cannot export Maven model", error); }
    }
    /** Effective compiler configuration overrides inherited properties, just as Maven compilation does. */
    private Map<String,Object> compiler(MavenProject project) throws Exception {
        Map<String,Object> result = new LinkedHashMap<>();
        org.apache.maven.model.Plugin compiler = null;
        for (org.apache.maven.model.Plugin plugin : project.getBuildPlugins()) {
            if ("maven-compiler-plugin".equals(plugin.getArtifactId())) { compiler=plugin; break; }
        }
        MavenSession evaluation = session.clone();
        evaluation.setCurrentProject(project);
        org.apache.maven.plugin.PluginParameterExpressionEvaluator evaluator = new org.apache.maven.plugin.PluginParameterExpressionEvaluator(evaluation,new org.apache.maven.plugin.MojoExecution(compiler == null ? new org.apache.maven.model.Plugin() : compiler,"compile","default-compile"));
        for (String name : new String[]{"release","source","target"}) {
            String expression = "${maven.compiler." + name + "}";
            if (compiler != null) {
                Object configuration=compiler.getConfiguration();
                // Maven's standard compile execution can override the plugin-level parameter.
                for (org.apache.maven.model.PluginExecution execution: compiler.getExecutions())
                    if ("default-compile".equals(execution.getId()) && execution.getConfiguration() instanceof org.codehaus.plexus.util.xml.Xpp3Dom)
                        configuration=org.codehaus.plexus.util.xml.Xpp3Dom.mergeXpp3Dom(new org.codehaus.plexus.util.xml.Xpp3Dom((org.codehaus.plexus.util.xml.Xpp3Dom)execution.getConfiguration()),configuration instanceof org.codehaus.plexus.util.xml.Xpp3Dom ? new org.codehaus.plexus.util.xml.Xpp3Dom((org.codehaus.plexus.util.xml.Xpp3Dom)configuration) : null);
                if (configuration instanceof org.codehaus.plexus.util.xml.Xpp3Dom) {
                    org.codehaus.plexus.util.xml.Xpp3Dom value=((org.codehaus.plexus.util.xml.Xpp3Dom)configuration).getChild(name);
                    if (value!=null && value.getValue()!=null && !value.getValue().trim().isEmpty()) expression=value.getValue().trim();
                }
            }
            Object evaluated=evaluator.evaluate(expression);
            String value=evaluated == null ? "" : evaluated.toString().trim();
            if (value.contains("${")) throw new IllegalArgumentException("Unresolved Java compiler parameter: " + name);
            result.put(name,value.startsWith("1.") ? value.substring(2) : value);
        }
        return result;
    }
    /** Source syntax and target bytecode can differ; their highest feature bounds the required JDK. */
    private static String newer(String source,String target) {
        return Integer.parseInt(source.isEmpty() ? "0" : source) >= Integer.parseInt(target.isEmpty() ? "0" : target) ? source : target;
    }
    /** Keep reactor edges as identities; only external artifacts are resolved to real repository files. */
    private List<Object> dependencies(MavenProject project, Set<String> reactor) throws Exception {
        List<Object> dependencies = new ArrayList<>();
        if (!project.getDependencies().isEmpty()) {
            DefaultDependencyResolutionRequest request = new DefaultDependencyResolutionRequest(project, session.getRepositorySession());
            // Reactor JARs need not exist during import. Their source/output model supplies the edge;
            // the same Resolver still collects their POMs and external transitive dependencies.
            request.setResolutionFilter((node, parents) -> {
                Artifact artifact = node.getArtifact();
                return artifact == null || !reactor.contains(artifact.getGroupId() + ":" + artifact.getArtifactId() + ":" + artifact.getVersion());
            });
            org.apache.maven.project.DependencyResolutionResult result = resolver.resolve(request);
            Set<String> included = new java.util.LinkedHashSet<>();
            for (org.eclipse.aether.graph.DependencyNode child : result.getDependencyGraph().getChildren())
                collect(child, reactor, included, dependencies, 0);
        }
        return dependencies;
    }
    /** Read the already mediated graph, including reactor nodes skipped only for binary acquisition. */
    private void collect(org.eclipse.aether.graph.DependencyNode node, Set<String> reactor,
            Set<String> included, List<Object> dependencies, int depth) throws Exception {
        if (node.getData().get(org.eclipse.aether.util.graph.transformer.ConflictResolver.NODE_DATA_WINNER) != null) return;
        Dependency dependency = node.getDependency();
        if (dependency == null) return;
        Artifact artifact = dependency.getArtifact();
        String id = artifact.getGroupId() + ":" + artifact.getArtifactId() + ":" + artifact.getVersion();
        if (!included.add(id + ":" + artifact.getExtension() + ":" + artifact.getClassifier())) return;
        if (depth > 128 || included.size() > 4096) throw new IllegalStateException("Maven dependency graph quota exceeded");
        Map<String,Object> resolved = new LinkedHashMap<>();
        resolved.put("id", id);
        resolved.put("scope", dependency.getScope());
        resolved.put("type", artifact.getExtension());
        resolved.put("classifier", artifact.getClassifier());
        if (reactor.contains(id)) resolved.put("module", id);
        else {
            if (artifact.getFile() == null || !artifact.getFile().isFile()) throw new IllegalStateException("Resolved Maven dependency has no file");
            resolved.put("file", artifact.getFile().getCanonicalPath());
        }
        dependencies.add(resolved);
        for (org.eclipse.aether.graph.DependencyNode child : node.getChildren()) collect(child, reactor, included, dependencies, depth + 1);
    }
    /** Future output directories are admitted through canonical ancestors, with no workspace escape. */
    private static String relative(Path root, Path path) throws Exception {
        Path checked = path.toFile().getCanonicalFile().toPath();
        if (!checked.startsWith(root)) throw new IllegalArgumentException("Maven model path escapes workspace");
        return root.relativize(checked).toString().replace('\\', '/');
    }
    /** Digests identify model generations independently from mutable process/session identities. */
    private static String hex(byte[] bytes) {
        StringBuilder text = new StringBuilder();
        for (byte value : bytes) text.append(String.format("%02x", value & 255));
        return text.toString();
    }
}
