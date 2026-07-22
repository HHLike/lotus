# Lotus 🪷

> OTTY 风格的原生 Linux 终端 —— 为多标签、项目工作区与 Agent CLI 场景打造。

[![License: MIT](https://img.shields.io/badge/License-MIT-pink.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-2024-orange.svg)](https://www.rust-lang.org/)
[![Platform](https://img.shields.io/badge/platform-Linux-blue.svg)](#)

Lotus 是独立的 **GUI 终端应用**（不是 TUI）。设计语言参考 [Otty](https://otty.sh/)：原生窗口、深色界面、侧边栏、多标签，并针对 Claude Code / Codex / Pi 等 Agent CLI 做了启动与状态增强。

仓库：https://github.com/HHLike/lotus

---

## 界面预览

```
┌─────────────────────────────────────────────────────────────────┐
│ 🪷 Lotus          📁 Lotus                               — ☐ ✕  │
├──────────┬──────────────────────────────────────────────────────┤
│ ▶ 终端   │  ● pi   ○ bash-2                            [+]     │
│ ✦ Agents ├──────────────────────────────────────────────────────┤
│ 🕘 历史  │                                                      │
│ ★ 书签   │     $ pi code                                        │
│ ⚙ 设置   │     … agent TUI …                                    │
│          │                                                      │
│ 项目     │                                                      │
│  • Lotus │                                                      │
│ 最近     │                                                      │
│  • ~/…   │                                                      │
└──────────┴──────────────────────────────────────────────────────┘
```

---

## 功能特性

### 终端核心
- 原生 GTK 窗口 + WebKitGTK 渲染（非 Electron）
- xterm.js 终端（真彩色、选区、链接）
- 多标签会话（创建 / 切换 / 关闭）
- PTY 全功能交互（bash、vim、htop、TUI Agent 等）
- 窗口缩放自动同步 cols/rows
- Shell Integration（命令开始/结束捕获、历史记录）

### 工作区
- **项目（Workspace）**：独立 cwd / 历史 / 书签
- **历史**：命令记录、搜索、一键重跑
- **书签 / 最近目录**：快速跳转

### Agent 友好
- **Agents 面板**：探测本机 `claude` / `codex` / `opencode` / `pi` / `aider` 等，一键新标签启动
- **Tab 徽章**：运行中 / 成功 / 失败状态
- **完成通知**：Agent CLI 单轮任务与普通命令通知可分别开关；后台 Agent 进程结束遵循安静通知策略（`notify-send`）
- 不抢占 Agent 自带输入框（Pi / Claude 等 TUI 全屏使用）

### 交互
- 自定义右键菜单（屏蔽浏览器默认菜单）
- `Ctrl+C` 复制 · `Ctrl+V` 粘贴 · `Ctrl+Z` 中断（SIGINT）
- 剪贴板走 GTK 系统剪贴板（WebKit 自定义协议下更可靠）
- 设置面板：主题、字体、字号、透明度、默认 Shell
- 通知设置：Agent CLI 完成通知默认开启，普通命令完成通知默认关闭

---

## 技术栈

| 层 | 技术 | 说明 |
|----|------|------|
| 窗口 / WebView | **gtk-rs 0.18** + **webkit2gtk 2.0** | 与系统 WebKitGTK 4.1 同路径 |
| 前端 | HTML / CSS / JS（无框架） | 轻量、易改 |
| 终端渲染 | **xterm.js**（本地 vendor） | 不依赖 CDN |
| PTY | **portable-pty** | 多 tab 进程池 |
| 后端 | **Rust**（edition 2024） | IPC、存储、打包 |

### 架构

```
┌──────────────────────────────────────┐
│     GTK Window + WebKitGTK WebView    │
│  ┌──────────────────────────────────┐│
│  │   HTML/CSS/JS + xterm.js         ││  UI / 终端渲染
│  └────────────┬─────────────────────┘│
│          IPC (JSON)                   │
└───────────────┼──────────────────────┘
                │
     ┌──────────▼──────────┐
     │  Rust Backend        │
     │  TermManager         │  多 tab PTY
     │  portable-pty        │  每个 tab 一个 shell
     │  storage / config    │  项目 · 历史 · 书签
     └──────────────────────┘
```

**数据流**
- 输入：xterm.js `onData` → IPC → `pty.write()`
- 输出：PTY reader 线程 → 批量缓冲 → `run_javascript` → xterm.js
- 前端资源：优先 `/usr/share/lotus/frontend`（deb），开发时用仓库内 `frontend/`

---

## 快速开始

### 运行时依赖（Ubuntu / Debian）

```bash
sudo apt install libwebkit2gtk-4.1-0 libgtk-3-0 libjavascriptcoregtk-4.1-0
# 可选：通知、字体
sudo apt install libnotify-bin fonts-jetbrains-mono
```

### 从源码构建

```bash
# 构建依赖
sudo apt install build-essential curl \
  libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev pkg-config

# Rust（若尚未安装）
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

git clone https://github.com/HHLike/lotus.git
cd lotus
cargo build --release

# 运行
./target/release/lotus
# 若 Wayland 下异常，可强制 X11：
GDK_BACKEND=x11 ./target/release/lotus
```

### 安装 deb 包

```bash
# 在本机构建
./packaging/build-deb.sh
# 产物：dist/lotus_<version>_<arch>.deb

sudo apt install ./dist/lotus_*.deb
lotus
```

> **注意（glibc）**  
> 在 Ubuntu 24.04 上打的包会链接较新的 glibc（如 `GLIBC_2.39`），  
> **不能**直接在 Ubuntu 22.04 等更老系统上运行。  
> 请在目标系统同代环境（或对应 Docker 镜像）中重新 `build-deb.sh`。

---

## 快捷键

| 快捷键 | 作用 |
|--------|------|
| `Ctrl+T` | 新建标签 |
| `Ctrl+W` | 关闭当前标签 |
| `Ctrl+Q` | 退出 |
| `Ctrl+1` … `9` | 切换到第 N 个标签 |
| `Ctrl+C` | 复制选区 |
| `Ctrl+V` | 粘贴 |
| `Ctrl+Z` | 中断当前进程（SIGINT） |
| `F12` / `Ctrl+Shift+I` | 开发者工具（诊断） |

---

## 配置

配置文件：`~/.config/lotus/config.toml`

```toml
theme = "lotus"
shell = "/bin/bash"          # 可选，默认 $SHELL
font = "JetBrains Mono"
font_size = 14
opacity = 1.0
agent_notifications_enabled = true
command_notifications_enabled = false
```

数据目录：`~/.local/share/lotus/`（项目、历史、书签、shell integration 脚本）

---

## 项目结构

```
lotus/
├── src/
│   ├── main.rs              # GTK 窗口 · WebView · 主循环 · IPC
│   ├── ipc.rs               # 前后端 JSON 协议
│   ├── pty.rs               # PTY spawn / 读写
│   ├── term/manager.rs      # 多 tab 管理 · OSC 解析
│   ├── shell_integration.rs # bash 集成脚本
│   ├── storage.rs           # 项目 / 历史 / 书签持久化
│   ├── config.rs            # TOML 配置
│   └── theme.rs             # 主题色板
├── frontend/
│   ├── index.html
│   ├── styles.css
│   ├── app.js
│   └── vendor/              # xterm.js 本地打包
├── packaging/
│   ├── build-deb.sh         # 一键打 deb
│   ├── lotus.desktop
│   └── lotus.svg
├── Cargo.toml
└── README.md
```

---

## 开发

```bash
# debug 运行（带日志）
RUST_LOG=info cargo run

# 测试
cargo test --bin lotus

# 重新打包
./packaging/build-deb.sh
```

前端修改后，开发模式直接读仓库内 `frontend/`；安装版读 `/usr/share/lotus/frontend/`。  
也可用环境变量覆盖：

```bash
LOTUS_FRONTEND=/path/to/frontend ./target/release/lotus
```

---

## 路线图（部分）

- [ ] 命令面板（模糊查找）
- [ ] 分屏（水平 / 垂直）
- [ ] 更完整的 Agent hooks（状态上报）
- [ ] 多发行版 CI 打包（22.04 / 24.04）
- [ ] 应用图标与主题扩展

---

## 许可证

[MIT](LICENSE)

---

## 致谢

- [Otty](https://otty.sh/) — 产品与交互灵感  
- [xterm.js](https://xtermjs.org/) — 终端渲染  
- [portable-pty](https://github.com/wez/wezterm) — PTY  
- GTK / WebKitGTK — 原生 Linux 窗口与 WebView  
