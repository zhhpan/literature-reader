# 文献阅读器 Linux Wayland 版

此归档面向 Ubuntu 24.04 x86_64 的 Wayland 会话。需要 GTK 4 和 Poppler 命令行工具，可用以下命令安装运行依赖和中文字体。

```bash
sudo apt install libgtk-4-1 poppler-utils xdg-utils fonts-noto-cjk
```

解压后在 `literature-reader` 目录运行 `./literature-reader`。程序会使用包内 Node.js。Poppler 的 `pdftotext` 需支持 `-tsv`。请保留程序、`bridge` 和 `runtime` 的相对位置。

如需加入桌面应用菜单，在解压后的目录运行以下命令。桌面会话需要能通过 `PATH` 找到 `~/.local/bin` 中的程序。

```bash
data_dir=${XDG_DATA_HOME:-"$HOME/.local/share"}
app_id=io.github.zhhpan.LiteratureReader
mkdir -p "$HOME/.local/bin"
ln -s "$(pwd -P)/literature-reader" "$HOME/.local/bin/literature-reader"
install -Dm644 "share/applications/$app_id.desktop" "$data_dir/applications/$app_id.desktop"
install -Dm644 "share/icons/hicolor/512x512/apps/$app_id.png" \
    "$data_dir/icons/hicolor/512x512/apps/$app_id.png"
```
