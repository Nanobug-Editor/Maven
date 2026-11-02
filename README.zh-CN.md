# Maven for Nanobug

版本 **0.1.19** 面向 2026-11-02 的 Nanobug 内部测试版，使用公开宿主 SDK 构建。保留安装 ID 和权限；包内附带的 Windows 原生辅助程序使用静态运行库。

[English](README.md)

独立的 Maven 工程导入与构建插件。Gradle、Lombok、JUnit 保持为其他独立插件。
当前为本地实现包，尚未发布到插件市场。

使用 `Nanobug.exe --plugin-package C:/Projects/RustProjects/Maven-for-Nanobug` 构建。
宿主提供公开 SDK 缓存，访客工程不引用宿主 checkout，也不附带 SDK 源码副本。
Windows x86-64 构建直接使用 Cargo、`javac`、`jar`、`tar`。运行需要协议 7 和
`language.lsp` 1.9。Java 分析、应用运行和调试另外需要提供 `java.project` 1.2 的
Java 插件；Maven 不实现这些语言功能。

通过 Nanobug 插件管理器安装 ZIP 并允许清单权限，在受信任的工作区打开 **Maven 工程** 面板，
选择工作区内 POM、填写逗号分隔的 profile，然后导入／刷新。导入通过真实 Maven 求值 reactor
并解析依赖，不编译、不运行应用。Java 语言提供者选择 `maven/analysis`。POM／`.mvn` 事件会
撤销旧上下文；修改构建输入后重新导入。失败或取消时保留带过期标记的描述模型，不能继续驱动分析和启动。

原生运行配置提供 **Maven 目标** 和 **Maven Java 应用**。目标使用逐项字面参数、环境变量和
工作目录，输出、停止、重跑进入原生会话。应用选择已导入的模块和主类，先执行 Maven reactor
`compile`，再把实际运行类路径及 JDK 交给 Java。调试沿用 Java 的断点、线程、调用栈及变量界面。

用户设置 `maven.home`、`maven.java` 可指定本机工具；无效覆盖明确报错。运行 Maven 时须在 PATH
已有 Java 8 或更新版本，或用 `maven.java` 指定。项目设置 `maven.repository` 覆盖本地仓库；
其他 settings、镜像和仓库选择由 Maven 负责。工程面板和运行配置均提供 profile 与离线模式，
标准 `.mvn/maven.config` 仍交由 Maven 读取。

固定的 Apache Maven 3.10.0 ZIP 已核对官方摘要；宿主在安装授权后解包到私有依赖目录。
不修改全局 PATH。Apache 发行的 LICENSE 和 NOTICE 保留在 `vendor/maven.zip` 中。

同一 reactor 共享 Java 分析会话，各模块的 main／test 工程仍保留独立类路径；仅加入已经解析且
当前可见的依赖，保留依赖仲裁、排除项及顺序。显式 release／source／target 及标准
`default-compile` 配置通过 Maven 表达式求值器导出。未声明的编译参数沿用 Java 的配置默认值；
不推断任意编译插件默认值、Toolchain 或自定义生命周期执行的等价约束。当前不宣称支持 Maven
Wrapper、Maven 4 或 `.mvn/jvm.config` 的 JVM 参数。导入模型上限为 48 KiB、Java 模块 64 个、
Java 分析图 32 KiB；超限明确报错。
