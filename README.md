# Lotus 🪷

> 一个 OTTY 风格的原生窗口终端应用，为 Linux 桌面打造。

Lotus 是一个独立的 GUI 终端应用（不是 TUI），参考 [Otty](https://otty.sh/) 的设计语言：原生窗口、深色高颜值、多标签、侧边栏、现代化交互。

## 截图（ASCII 预览）
```
┌─────────────────────────────────────────────────────────────────┐
│ 🪷 Lotus          [ workspace ]                          — ☐ ✕   │ ← 标题栏
├──────────┬──────────────────────────────────────────────────────┤
│ ▶ 终端   │  ◉ tab1   ○ tab2                            [+]     │ ← 标签栏
│ 🕘 历史  ├──────────────────────────────────────────────────────┤
│ ★ 书签   │                                                      │
│ ⚙ 设置   │      [xterm.js 终端区 - bash 交互]                    │
│          │      $ ls -la                                        │
│ 最近     │      drwxr-xr-x  src/  frontend/  Cargo.toml         │
│ • lotus  │      $ echo hello                                    │
│ • docs   │      hello                                           │
│ 书签     │                                                      │
│ • ~/work │                                                      │
└──────────┴──────────────────────────────────────────────────────┘
```

## 技术栈

| 层 | 技术 | 说明 |
|----|------|------|
| 原生窗口 | **Wry 0.55** + **tao 0.33** | WebKitGTK webview 包装 |
| 前端 | **HTML/CSS/JS**（无框架） | 轻量，后期可升级 |
| 终端渲染 | **xterm.js 5.5** + WebGL | VS Code/Tabby 同款 |
| PTY | **portable-pty 0.9** | wezterm 出品，复用自 v1 |
| 后端 | **Rust 2024 edition** | 管理 PTY 池 + IPC |

## 架构

```
┌──────────────────────────────────────┐
│        Wry Window (WebKitGTK)         │
│  ┌──────────────────────────────────┐│
│  │     HTML/CSS/JS + xterm.js        ││  ← 渲染 UI 和终端
│  └────────────┬─────────────────────┘│
│           IPC (JSON)                  │
└───────────────┼──────────────────────┘
                │
     ┌──────────▼──────────┐
     │  Rust Backend        │
     │  Terminal Manager    │  ← 多 tab PTY 池
     │  portable-pty        │  ← 每个 tab 一个 bash
     └──────────────────────┘
```

**数据流**：
- 键盘：xterm.js → `onData` → IPC → `pty.write()`
- 输出：bash → reader 线程 → mpsc → 批量缓冲(16ms) → `evaluate_script` → xterm.js
- 资源：`lotus://` 自定义协议加载本地前端文件（避开 file:// URL 的 bug）

## 快速开始

### 依赖
- Ubuntu 24.04 / Debian 12（或其他带 WebKitGTK 4.1 的发行版）
- 系统包：`libwebkit2gtk-4.1-0` `libgtk-3-0`（运行时）
- 构建：Rust 1.74+（rustup stable）

### 构建
```bash
# 安装构建依赖（首次）
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev

# 编译
cargo build --release
# 二进制：target/release/lotus (2.5MB)
```

### 运行
```bash
# Wayland 默认就能跑；某些环境下强制 X11 更稳：
GDK_BACKEND=x11 ./target/release/lotus
```

### 打包 deb
```bash
./packaging/build-deb.sh
# 产物：dist/lotus_<version>_<arch>.deb

# 安装
sudo apt install ./dist/lotus_0.2.0_amd64.deb
# 启动：应用菜单搜 Lotus，或命令行 lotus
```

## 功能

### ✅ 已实现（v0.2）
- 原生独立窗口（不是 TUI）
- OTTY 风格 UI：标题栏 + 侧边栏 + 标签栏 + 终端区
- xterm.js 终端渲染（真彩色、光标闪烁、字体抗锯齿）
- 多 tab 终端会话（创建/切换/关闭）
- PTY 交互（bash 命令、vim、htop 等全功能）
- 窗口缩放自适应（cols/rows 自动更新）
- Lotus 粉深色主题
- 快捷键：`Ctrl+T` 新 tab、`Ctrl+W` 关 tab、`Ctrl+Q` 退出、`Ctrl+数字` 切换

### 🚧 规划中
- `Ctrl+K` 命令面板（模糊查找）
- 分屏（水平/垂直 split）
- 工作区（多工作区独立配置）
- OSC 标题序列解析（tab 自动显示当前命令）
- 命令块化（OTTY 招牌的 block UI）
- 应用图标 + .desktop 集成 + deb 打包
- 离线 xterm.js（本地打包，不依赖 CDN）

## 快捷键

| 键 | 动作 |
|----|------|
| `Ctrl+T` | 新建标签 |
| `Ctrl+W` | 关闭当前标签 |
| `Ctrl+Q` | 退出应用 |
| `Ctrl+1`~`9` | 切换到第 N 个标签 |

## 配置

`~/.config/lotus/config.toml`：
```toml
theme = "lotus"          # 主题名
shell = "/bin/bash"      # 可选，缺省用 $SHELL 或 bash
```

## 项目结构
```
lotus/
├── src/
│   ├── main.rs           # Wry 窗口 + 事件循环 + IPC 桥
│   ├── ipc.rs            # JSON 消息协议（ClientMessage/ServerMessage）
│   ├── pty.rs            # PTY spawn + reader 线程（复用自 v1）
│   ├── term/
│   │   ├── mod.rs
│   │   └── manager.rs    # 多 tab PTY 池管理
│   ├── config.rs         # TOML 配置
│   └── theme.rs          # 主题配色（RGB 元组，无 UI 框架耦合）
└── frontend/
    ├── index.html        # OTTY 风格布局
    ├── styles.css        # 深色主题 + CSS 变量
    └── app.js            # xterm.js 管理 + IPC + tab UI
```

## 许可证

MIT
