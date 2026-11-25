# 文献阅读器

一个使用 Rust 的免费双语桌面阅读器，支持 Windows 原生界面和 Linux 原生 Wayland。导入带文字层的论文 PDF 后，按阅读顺序显示原文和对应的简体中文译文。源码采用 MIT 许可证。

## 安装与阅读

1. 下载并运行 Windows x64 安装包。在安装向导的「选择安装位置」页面可更改目录，覆盖安装时默认填入原目录。首次安装会下载并校验所需的运行库。 [GitHub Releases](https://github.com/zhhpan/literature-reader/releases/latest)
2. 从开始菜单打开「Literature Reader」，完成 CodeBuddy 登录。进入后可从「最近阅读」打开已有记录，单击「打开 PDF」，或将 PDF 拖到窗口。
3. 在双语视图连续滚动阅读整篇论文。标题和译名居中，作者信息保留原文，小标题和正文使用不同字样。拖动鼠标选中文字后按 Ctrl+C 复制，也可右键复制整段或切到「PDF 原页」核对版面。
4. 点正文区块的引用可跳转到对应的文末条目。带 DOI 或来源网址的条目可以打开论文出处。
5. 阅读位置会自动保存在本机。下次打开内容相同的 PDF 时，双语视图会回到上次读到的段落，原页视图会恢复到上次的页码和页内位置。

## 更新

Windows 启动时会在后台读取 GitHub Releases 的公开更新清单，也可以从底部状态栏手动「检查更新」。较早的发布可通过 GitHub API 查询。点「更新到」并确认一次后，阅读器会下载并校验安装包、保存阅读位置，按已有安装目录和选项安装。升级时直接复用通过 SHA-256 校验的 Node.js、Poppler 和翻译 SDK，成功后重新打开阅读器。

Linux x86_64 版可在 Wayland 合成器会话中直接运行。依赖 GTK 4 和 Poppler，归档内含 Node.js。下载新版归档后解压启动，阅读数据存放在 XDG 数据目录。 [Linux 使用说明](installer/linux/README.md)

## 账号登录和翻译

阅读器使用腾讯的 CodeBuddy Agent SDK。国内 WorkBuddy 和 CodeBuddy 同账号共享积分，可选模型以 CodeBuddy 对该账号实际返回的列表为准。 [积分说明](https://www.workbuddy.cn/docs/workbuddy/Pricing)

1. 在入口点「登录 CodeBuddy」，使用国内 WorkBuddy 的同一账号完成授权。若默认浏览器支持小窗，应用会在独立登录窗口中打开官方授权页。小窗启动失败时，可以在入口复制授权链接。登录成功后，应用会刷新可用模型。
2. 模型菜单显示 CodeBuddy 返回的积分消耗倍率。点「查看官方用量」可到 CodeBuddy 个人中心查看剩余积分。选择模型后，当前论文会自动按段翻译。阅读器会记住这次选择，重启后该模型仍可用时，打开 PDF 会自动翻译。
3. 译文随模型输出逐步显示。翻译时，顶部模型按钮旁显示估算的 token 输出速率，底部状态栏显示段落进度。点「继续翻译」可接着翻译尚未完成的段落。
4. 底部的文档积分按 PDF 内容累加 CodeBuddy 每轮上报的消耗。记录保存在本机，下次打开同一 PDF 时恢复。已缓存的译文直接复用。有缺失的用量会标为部分记录，升级前的消耗无法补记。 [每轮积分字段说明](https://www.codebuddy.cn/docs/cli/stream-json-turn-usage)

已完成的段落会按 PDF 内容和模型保存在本机。再次打开相同 PDF 时会先恢复译文，随后只提交尚未翻译的段落。已保存的译文也可离线阅读。

登录凭据由 CodeBuddy SDK 缓存和管理，使用者无需填写 API Key。阅读器重启后会在后台恢复有效登录并加载模型列表。点右上角的圆形账号图标可以查看登录状态和重新登录。 [SDK 文档](https://www.codebuddy.cn/docs/cli/sdk)

PDF 解析和原页查看可以离线使用。翻译需要联网，应用向模型发送待译文字并按段落编号回填。

同一份论文的翻译任务复用一个接入进程，短会话限制前面批次原文和译文的重发。支持关闭可选思考的模型会直接翻译。批量结果缺失时只补翻缺少的段落，自动补翻最多一次。停止翻译、切换模型或打开另一份论文时会结束当前任务。

## 识别范围

- 英文正文会根据文中拼写和本地 SCOWL 英语词表恢复 PDF 行尾断词。拼法有歧义时保留连字符。 [词表版权与许可](src/data/SCOWL-Copyright.txt)
- 识别标题、作者、章节、正文、图表题注、参考文献和独立公式，恢复常见单栏、双栏、三栏及同页通栏转分栏的阅读顺序。作者及单位保留原文，图表与对应题注归在一起，仅翻译题注。
- 同一层级的连续列表集中呈现，先显示完整原文列表，再在下方显示译文列表。
- 参考文献统一展示，按独立的学术书目规则翻译。标题译成中文，作者、发表信息与原始链接随条目保留。自动匹配数字编号及可唯一确定的作者年份引用。
- 阅读器使用 Latin Modern 字体原生排版能从 PDF 文字层恢复的公式，支持上下标、分式、根式和常用数学符号。结构难以恢复的独立公式保留 PDF 原貌。翻译时保护识别到的行内公式，原文和译文均复用公式内容。 [字体版权与许可](src/data/latex-fonts/README.md)
- 图表保留 PDF 原貌。矢量插图和表格的区域依赖题注和版面推断。
- 扫描件可以查看 PDF 原页。当前版本还没有 OCR，也没有公式转 LaTeX。复杂多栏和异常文字层可能影响分段与阅读顺序。译文质量取决于所选模型。

## 从源码构建

使用 Windows、Rust 1.88 或更新版本、Visual Studio C++ Build Tools 和 Node.js 20 或更新版本。先安装锁定的 SDK 依赖，再构建桌面程序。

```powershell
npm ci --prefix bridge
cargo build --release --locked
```

产物位于 `target\release\literature-reader.exe`。直接从源码目录运行时，将 Node.js 和 [Poppler](https://poppler.freedesktop.org/) 加入 `PATH`。Poppler 的 `pdftotext` 需支持 `-tsv` 参数。PDF 提取、原页渲染和翻译在后台运行。界面只绘制视口中的区块，并按需缓存图像。

### 构建安装包

使用 Inno Setup 6.7.4。先构建 x64 版阅读器，再执行安装包脚本。将 `-Compiler` 参数改为本机 `ISCC.exe` 的路径。

```powershell
$env:RUSTFLAGS = '-C target-feature=+crt-static'
cargo build --release --locked --target x86_64-pc-windows-msvc
.\installer\build.ps1 -Compiler 'C:\Program Files (x86)\Inno Setup 6\ISCC.exe' -AppExecutable 'target\x86_64-pc-windows-msvc\release\literature-reader.exe'
```

安装包输出到 `target\release\LiteratureReader-Setup-<版本>-x64.exe`。

### GitHub Actions 打包

在 GitHub 的 Actions 页面手动运行「Desktop packages」可下载预览安装包。推送与 `Cargo.toml` 版本一致的标签会构建 Windows x64 安装包和 Linux Wayland x86_64 归档，在校验 SHA-256 后与更新清单一同发布到 GitHub Releases。 [工作流](.github/workflows/windows-installer.yml)

### 发布签名版

在 Windows 当前用户的个人证书存储 `Cert:\CurrentUser\My` 中准备受信任的代码签名证书及其私钥，并使用 Windows SDK 提供的 SignTool。构建时传入证书指纹。脚本会给阅读器、安装程序和卸载器添加带时间戳的 Authenticode 签名，并校验主程序和安装包的签名。

```powershell
.\installer\build.ps1 -Compiler 'C:\Program Files (x86)\Inno Setup 6\ISCC.exe' `
    -AppExecutable 'target\x86_64-pc-windows-msvc\release\literature-reader.exe' `
    -SigningCertificateThumbprint '<证书指纹>'
```

如果 SignTool 未加入 `PATH`，通过 `-SignTool` 指定 `signtool.exe` 的路径。签名让用户可以验证发布者。SmartScreen 还会评估文件和发布者声誉，新发布的签名版仍可能显示提示。 [微软的 SmartScreen 发布说明](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/smartscreen-reputation)
