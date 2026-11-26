# 文献阅读器 · Tern 插件

在 Tern 里以原生区块（block）阅读「文献阅读器」生成的双语学术 PDF 视图：
原文、简体中文翻译、公式标记、图表与原文页对照。

插件本身只包含 Tern 包；PDF 解析、翻译与渲染由 Rust 编写的
`--tern` 无头后端完成。后端在首次打开时下载到插件数据目录，与桌面版互不依赖。

## 安装

需要 Tern 0.9 或更新版本。插件自带运行时，**不需要安装桌面版，也不需要设置可执行程序**：
首次打开时会从 GitHub Releases 下载一次精简后端（Windows x64，约 82 MB），
之后完全离线可用。

从 GitHub 安装（推荐；需要 `git`）：

```sh
tern plugin install github.com/zhhpan/literature-reader/plugins/literature-reader
```

开发时可以直接链接本地目录（改动立即生效）：

```sh
tern plugin link "E:/Literature reader/plugins/literature-reader"
```

查看加载状态与问题：

```sh
tern plugin list
```

`tern plugin reload` 会重新加载宿主半（`host.luau`）与窗口半（`window.luau`）。
卸载：`tern plugin unlink literature-reader`（或 `tern plugin remove literature-reader`，
若当初是 `install` 的）。

## 登录 CodeBuddy

翻译和模型列表使用 CodeBuddy 账号。三种入口等价：

- 命令面板：**文献阅读器：登录 CodeBuddy**；
- 区块底部 dock 的 **登录** 按钮（未登录时出现）；
- 最近阅读页的「翻译需要 CodeBuddy 登录」卡片。

登录会打开浏览器授权页；完成后插件自动刷新模型列表，不需要重启 Tern。
凭据由后端 `literature-reader --tern login` 保存，插件只读取登录状态和模型。

首次使用前，还可在命令面板运行：

- **文献阅读器：设置可执行文件…** — 仅离线包或自定义构建需要；
- **文献阅读器：自动翻译开/关** — 打开文献后是否自动翻译（默认关闭；开启后才会按模型自动开始，随时可在区块 dock 点「翻译」手动开始）。

## 打开方式

- 命令面板：**文献阅读器：打开标签页** — 在新标签页打开，显示最近阅读；
- 命令面板：**文献阅读器：打开 PDF…** — 输入路径后在新标签页打开；
- 文件窗格：对任意 `*.pdf` 选择 **打开方式 → 文献阅读器**（由清单的
  `files = ["*.pdf"]` 提供）。

区块的按键：`t` 切换译文，`p` 切换双语/原页，`[` / `]` 在原页模式翻页，
`r` 重新读取当前文献。

## 运行时解析顺序

每个区块解析一次，按顺序取第一个通过 `tern.fs.exists` 校验的候选：

1. 插件键值 `exe` 或环境变量 `LITERATURE_READER_EXE`（仅开发者/离线覆盖）；
2. 插件自带运行时：`<plugin.data>/backend/literature-reader.exe`；
3. 开发检出：`<plugin.dir>/../../target/release/literature-reader.exe`，
   然后 `.../debug/literature-reader.exe`。

正常安装只会走第 2 项。缺失时区块显示「阅读内核」卡片，自动下载、校验 SHA-256
并解压到插件数据目录；失败可点「重试」。不再探测桌面版安装目录或 PATH。

## 功能

- **双语正文**：标题、作者、章节标题、段落、公式、图表题注、参考文献，
  原文在上、译文在下（译文略暗、带左侧标记）。
- **列表组**：同一列表的所有原文连续显示，随后连续显示所有译文，不重复。
- **图表**：`figure` / `table` 区块由后端逐张渲染为 PNG，与配对题注一起显示；
  渲染失败时保留题注并显示占位提示。
- **公式**：后端在 Rust 中把 PDF 的公式位置还原成 TeX；插件用 Tern 原生 math 排版
  （display 公式居中，行内公式随正文流动），不再显示为代码或等宽文本。
- **图与表**：优先使用页面内嵌的位图；纯矢量图（例如 FreeBSD 命名空间表）按文本网格
  推断区域后由 Rust 从原始页裁剪渲染，图中标注不会被当成正文段落。
- **原页模式**：显示当前页位图与 `第 n / 总页数 页`，一次只生成一页，切页缓存。
- **翻译**：选择模型后运行 `--tern translate`；区块每 600 ms 读取 `state.json`，
  实时刷新进度；「停止」写入 `.cancel` 文件，后台进程自行收尾，不会被杀掉。
- **自动翻译（默认关闭）**：仅在命令面板开启后，且已登录、`preferred_model`
  存在、未译完时，快照与模型都到达后自动开始；手动停止或失败后不会自动重试。
  dock 的「翻译」按钮始终可用，不受此开关影响。
- **模型与登录**：模型列表显示名称与倍率；未登录时可发起 CodeBuddy 登录，
  轮询显示登录链接并支持复制到剪贴板。
- **最近阅读**：无参数打开时运行 `--tern list`，点击条目即可加载。

## 后端契约

插件调用的命令（全部原子写文件，失败时写 `{"schema":1,"error":"…"}` 并退出 1）：

```
<exe> --tern list <out.json>
<exe> --tern state <pdf> <out.json>
<exe> --tern translate <pdf> <out.json> <model-id>
<exe> --tern models <out.json>
<exe> --tern login <out.json>
<exe> --tern image <pdf> page <page-1-based> <out.png>
<exe> --tern image <pdf> block <block-index-0-based> <out.png>
```

每个区块独占 `<plugin.data>/panes/<pane>/`：

```
state.json  state.json.lock  state.json.cancel
block-<i>.png  page-<n>.png
```

`save` 只持久化 `{ path, mode, page, showZh }`；快照与图片 blob 不保存。

## 已知限制

- 公式的 TeX 来自 PDF 文字位置还原（上下标、分式、√ 等可还原的常见结构）；
  矩阵、分段函数等复杂版式可能退化为单行 TeX 或原文。
- 区块图片受 Tern 图像解码上限（每边 4096 px）约束；超大图表会显示占位。
- 原页位图按页生成并缓存，首次切到某页会有短暂等待。
- 翻译进行中不允许切换到另一篇文献：插件会先请求停止当前任务，避免多个进程
  写同一个 `state.json`。
- 进程崩溃可能留下 `state.json.lock`；点「停止」后约 12 秒会自动清理并提示
  「翻译进程已停止」。
- Tern 区块不会恢复桌面阅读器保存的阅读位置，也不会保存自己的滚动位置。
- 登录/模型列表依赖网络，`--tern models` 最坏可能耗时约一分钟；期间界面显示
  「正在读取模型…」。
- `auto_translate` 只在快照与模型都加载完成后判定一次；登录成功后仍会再判定。

## 开发

```
plugins/literature-reader/
├── plugin.toml      # 清单：id、窗口/宿主入口、区块声明、样式
├── host.luau        # 区块状态机：后端进程、轮询、图片、登录、按键与事件
├── window.luau      # 命令面板命令（新标签页、打开路径、设置 exe、自动翻译开关）
├── styles.css       # 全部位于 [data-surface='plugin.literature-reader.reader'] 下
└── lib/
    ├── backend.luau # 可执行文件解析、启动错误说明
    ├── render.luau  # 纯视图构造（文档、最近阅读、模型、登录、原页、dock）
    └── util.luau    # 有界读文件、JSON、UTF-8 截断、路径工具
```

本地验证（不依赖 Rust 构建）：

```sh
iso=$(mktemp -d)
printf '{"plugins":true}\n' > "$iso/settings.json"
TERN_CONFIG_DIR="$iso" tern plugin link "…/plugins/literature-reader"
TERN_CONFIG_DIR="$iso" TERN_DAEMON_SOCKET='\\.\pipe\tern-lit-test' tern plugin list --json
```

测试自动翻译时先写入插件键值 `auto_translate = true`（位于
`$iso/plugin-data/literature-reader/kv.json`）；默认关闭，避免测试打开真实账号。
