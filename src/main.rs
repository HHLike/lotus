//! Lotus 🪷 —— OTTY 风格的独立窗口终端应用
//!
//! 架构（v0.3，纯 webkit2gtk 路线，绕过 wry/tao 的 X11 foreign window 白屏问题）：
//! - gtk-rs 创建原生窗口
//! - webkit2gtk-rs 创建 WebView（和 yelp 走相同路径，渲染可靠）
//! - 前端 HTML/CSS/JS + xterm.js（本地 file:// 加载）
//! - IPC：UserContentManager.register_script_message_handler（JS→Rust）+ run_javascript（Rust→JS）
//! - 后端 Rust 管理 PTY（复用 pty.rs）+ 多 tab

mod config;
mod ipc;
mod pty;
mod shell_integration;
mod storage;
mod term;
mod theme;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{Context, Result};
use gtk::prelude::*;
use log::{error, info, warn};
// webkit2gtk 的类型通过 pub use 暴露到 crate root（需要 features v2_6 等）
use webkit2gtk::{
    JavascriptResult, UserContentManager, UserContentManagerExt, WebContext, WebView, WebViewExt,
    SettingsExt, UserContentInjectedFrames, UserScript, UserScriptInjectionTime,
};

use crate::config::Config;
use crate::ipc::{
    AgentInfoPayload, BookmarkEntryPayload, ClientMessage, ConfigPayload, HistoryEntryPayload,
    ProjectPayload, ServerMessage, ThemePayload,
};
use crate::storage::{
    now_ts, BookmarkStore, HistoryEntry, HistoryStore, ProjectStore, SessionStore, TabSession,
};
use crate::term::{TermEvent, TermManager};
use crate::theme::{rgb_to_css, Theme};

/// 应用共享状态
struct AppState {
    /// 终端 Manager（多 tab PTY 池）
    manager: Option<TermManager>,
    /// TermEvent 接收端（PTY 输出汇总）
    event_rx: Option<std::sync::mpsc::Receiver<TermEvent>>,
    /// 给前端待发送的消息队列（批量缓冲）
    pending_outputs: Vec<ServerMessage>,
    /// 主题
    theme: Theme,
    /// 配置（设置面板读写）
    config: Config,
    /// 首个 tab 是否已创建（含会话恢复）
    first_tab_created: bool,
    // ===== 项目（Workspace）=====
    /// 项目元数据存储
    projects: ProjectStore,
    /// 当前项目 id
    current_project_id: u32,
    /// 当前项目的历史（快照，切换项目时刷新）
    history: HistoryStore,
    /// 当前项目的书签（快照）
    bookmarks: BookmarkStore,
    /// 最近访问的目录（缓存，每次历史更新时重算）
    recents: Vec<String>,
    /// tab 当前正在跑的命令（用于时长统计 / agent 通知）
    /// tab_id -> (cmd, started_at)
    running_cmds: HashMap<u32, (String, Instant)>,
    /// 每项目当前激活的 tab_id（会话恢复 / 切换项目用）
    active_tab_by_project: HashMap<u32, u32>,
}

const FRAME_RESIZE_MARGIN: f64 = 6.0;
const TITLE_BAR_HEIGHT: f64 = 36.0;
const TITLE_CONTROLS_WIDTH: f64 = 104.0;
const DRAG_THRESHOLD: f64 = 4.0;

#[derive(Debug, Clone, Copy)]
struct PendingDrag {
    local_x: f64,
    local_y: f64,
    root_x: i32,
    root_y: i32,
    time: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameHit {
    Resize(gtk::gdk::WindowEdge),
    Drag,
    Client,
}

fn frame_hit(x: f64, y: f64, width: f64, height: f64, maximized: bool) -> FrameHit {
    if !maximized {
        let left = x < FRAME_RESIZE_MARGIN;
        let right = x >= width - FRAME_RESIZE_MARGIN;
        let top = y < FRAME_RESIZE_MARGIN;
        let bottom = y >= height - FRAME_RESIZE_MARGIN;

        let edge = match (left, right, top, bottom) {
            (true, _, true, _) => Some(gtk::gdk::WindowEdge::NorthWest),
            (_, true, true, _) => Some(gtk::gdk::WindowEdge::NorthEast),
            (true, _, _, true) => Some(gtk::gdk::WindowEdge::SouthWest),
            (_, true, _, true) => Some(gtk::gdk::WindowEdge::SouthEast),
            (_, _, true, _) => Some(gtk::gdk::WindowEdge::North),
            (_, _, _, true) => Some(gtk::gdk::WindowEdge::South),
            (true, _, _, _) => Some(gtk::gdk::WindowEdge::West),
            (_, true, _, _) => Some(gtk::gdk::WindowEdge::East),
            _ => None,
        };
        if let Some(edge) = edge {
            return FrameHit::Resize(edge);
        }
    }

    let controls_start = (width - TITLE_CONTROLS_WIDTH).max(0.0);
    if y < TITLE_BAR_HEIGHT && x < controls_start {
        FrameHit::Drag
    } else {
        FrameHit::Client
    }
}

fn drag_threshold_exceeded(start_x: f64, start_y: f64, x: f64, y: f64) -> bool {
    let delta_x = x - start_x;
    let delta_y = y - start_y;
    delta_x * delta_x + delta_y * delta_y >= DRAG_THRESHOLD * DRAG_THRESHOLD
}

fn preferred_gdk_backend(
    explicit_backend_is_set: bool,
    session_type: Option<&str>,
    desktop: Option<&str>,
    display: Option<&str>,
) -> Option<&'static str> {
    let is_wayland = session_type.is_some_and(|value| value.eq_ignore_ascii_case("wayland"));
    let is_ukui = desktop.is_some_and(|value| {
        value
            .split(':')
            .any(|part| part.eq_ignore_ascii_case("ukui"))
    });
    let x11_is_available = display.is_some_and(|value| !value.trim().is_empty());

    if !explicit_backend_is_set && is_wayland && is_ukui && x11_is_available {
        Some("x11")
    } else {
        None
    }
}

fn configure_gdk_backend() {
    let explicit_backend_is_set = std::env::var_os("GDK_BACKEND").is_some();
    let session_type = std::env::var("XDG_SESSION_TYPE").ok();
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").ok();
    let display = std::env::var("DISPLAY").ok();
    if let Some(backend) = preferred_gdk_backend(
        explicit_backend_is_set,
        session_type.as_deref(),
        desktop.as_deref(),
        display.as_deref(),
    ) {
        // SAFETY: main() invokes this before GTK, logging, or any worker thread starts.
        unsafe { std::env::set_var("GDK_BACKEND", backend) };
    }
}

fn main() -> Result<()> {
    configure_gdk_backend();

    // 初始化日志
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .try_init();
    info!("Lotus GUI 启动（webkit2gtk 路线）");

    // 加载配置 + 主题
    let config = Config::load();
    let theme = Theme::by_name(&config.theme);
    let shell = config.resolve_shell();
    let default_cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "/".to_string());
    info!("shell = {}, cwd = {}, font = {} {}, opacity = {}", shell, default_cwd, config.font, config.font_size, config.opacity);

    // 安装 shell 集成脚本（命令捕获的基石）
    let init_file = match shell_integration::install() {
        Ok(p) => {
            info!("shell integration 已安装：{}", p.display());
            Some(p)
        }
        Err(e) => {
            warn!("shell integration 安装失败（命令历史功能将不可用）：{}", e);
            None
        }
    };

    // 加载项目存储（首次启动自动创建默认项目 + 迁移旧数据）
    let projects = ProjectStore::load();
    // 按 config.last_project_id 选当前项目（不存在则用第一个）
    let current_project_id = config
        .last_project_id
        .and_then(|id| if projects.get(id).is_some() { Some(id) } else { None })
        .or_else(|| projects.list().first().map(|m| m.id))
        .unwrap_or(1);
    let current_meta = projects.get(current_project_id).cloned();
    let project = projects.load_project(current_project_id);
    let (history, bookmarks, project_cwd) = match project {
        Some(p) => {
            // 展开 ~/...，避免 PTY 把字面量 ~ 当路径
            let cwd = expand_user_path(&p.meta.cwd);
            (p.history, p.bookmarks, cwd)
        }
        None => (HistoryStore::default(), BookmarkStore::default(), default_cwd.clone()),
    };
    let recents = history.recent_dirs();
    info!(
        "当前项目 id={}, name={:?}, cwd={}, 历史 {} 条，书签 {} 个",
        current_project_id,
        current_meta.as_ref().map(|m| &m.name),
        project_cwd,
        history.entries.len(),
        bookmarks.entries.len()
    );

    // GTK 必须在主线程最早初始化
    gtk::init().context("gtk::init 失败")?;

    // 清空 WebKitGTK 磁盘缓存，确保开发期 file:// 下的 app.js 修改立即生效。
    // WebKitGTK 默认会缓存 file:// 资源（尤其是 JS 字节码缓存），导致前端改了不生效。
    if let Ok(home) = std::env::var("HOME") {
        for cache_sub in &[".cache/webkitgtk-4.1", ".local/share/webkitgtk-4.1"] {
            let cache_dir = std::path::PathBuf::from(&home).join(cache_sub);
            if cache_dir.exists() {
                if let Err(e) = std::fs::remove_dir_all(&cache_dir) {
                    warn!("清空 WebKit 缓存 {} 失败: {}", cache_dir.display(), e);
                }
            }
        }
    }

    // 创建 PTY 事件 channel + Manager（用当前项目的 cwd 作为默认目录）
    let (event_tx, event_rx) = std::sync::mpsc::channel::<TermEvent>();
    let manager = TermManager::new(shell, project_cwd.clone(), event_tx, init_file);

    // 应用状态（Arc<Mutex> 给 IPC 闭包用）
    let state = Arc::new(Mutex::new(AppState {
        manager: Some(manager),
        event_rx: Some(event_rx),
        pending_outputs: Vec::new(),
        theme: theme.clone(),
        config: config.clone(),
        first_tab_created: false,
        projects,
        current_project_id,
        history,
        bookmarks,
        recents,
        running_cmds: HashMap::new(),
        active_tab_by_project: HashMap::new(),
    }));

    // ====== 创建 GTK 窗口 ======
    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("Lotus 🪷");
    // 前端已经提供自绘标题栏，关闭窗口管理器的原生装饰以避免双标题栏。
    window.set_decorated(false);
    window.set_default_size(1100, 720);
    window.connect_destroy(|_| {
        info!("窗口关闭，退出 GTK 主循环");
        gtk::main_quit();
    });

    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
    window.add(&vbox);

    // ====== 创建 WebKit WebView ======
    // WebContext + UserContentManager（用于 IPC）
    let _web_context = WebContext::default();
    let content_manager = UserContentManager::new();

    // 注册 IPC：JS 调用 window.webkit.messageHandlers.ipc.postMessage(msg) → 触发回调
    // 我们注入一个 window.ipc.postMessage 桥，让前端代码和 wry 版本兼容
    let state_for_ipc = state.clone();
    let window_for_ipc = window.clone();
    content_manager.register_script_message_handler("ipc");
    content_manager.connect_script_message_received(None, move |_m, msg: &JavascriptResult| {
        // 提取 JS 传来的字符串
        if let Some(js_value) = msg.js_value() {
            let body = js_value.to_string();
            handle_client_message(&body, &state_for_ipc, &window_for_ipc);
        }
    });

    // webkit2gtk-rs 2.0 的 WebView 没有 new_with_context_and_user_content_manager
    // 用 with_user_content_manager 创建（context 通过其他方式关联）
    let webview = WebView::with_user_content_manager(&content_manager);

    // WebView 设置（消除 settings() 方法歧义：用 WebViewExt trait 限定）
    {
        let settings: Option<webkit2gtk::Settings> = WebViewExt::settings(&webview);
        if let Some(settings) = settings {
            settings.set_enable_javascript(true);
            settings.set_enable_developer_extras(true);
            settings.set_javascript_can_open_windows_automatically(true);
            // 开发期：禁用磁盘缓存 + 关闭离线 Web 应用缓存，确保 file:// 下的
            // app.js 修改能立即生效（避免 WebKitGTK 用旧缓存导致调试时改了不生效）
            settings.set_enable_offline_web_application_cache(false);
            // 注意：webkit2gtk 0.18 没有直接的 set_cache_model，缓存通过
            // FRESHNESS（file:// 自动 stat）+ 下面启动时清空缓存目录来保证
        }
    }

    // 屏蔽 WebKit 默认右键菜单（后退/刷新/检查元素等与应用无关的项）
    // 返回 true = 已处理，不展示原生菜单；前端用自定义 context menu 替代
    webview.connect_context_menu(|_wv, _menu, _event, _hit| true);

    // 无边框窗口的原生拖动/缩放。直接使用 GDK 事件的 root 坐标与时间戳，
    // 避免 DOM screenX/screenY 在 HiDPI、多屏和不同后端下发生坐标偏差。
    webview.add_events(
        gtk::gdk::EventMask::BUTTON_PRESS_MASK
            | gtk::gdk::EventMask::BUTTON_RELEASE_MASK
            | gtk::gdk::EventMask::POINTER_MOTION_MASK,
    );
    let pending_drag = std::rc::Rc::new(std::cell::RefCell::new(None::<PendingDrag>));
    let consume_release = std::rc::Rc::new(std::cell::Cell::new(false));
    let window_for_frame = window.clone();
    let pending_drag_for_press = pending_drag.clone();
    let consume_release_for_press = consume_release.clone();
    webview.connect_button_press_event(move |widget, event| {
        if event.button() != 1 {
            return gtk::glib::Propagation::Proceed;
        }

        let (x, y) = event.position();
        let (root_x, root_y) = event.root();
        let allocation = widget.allocation();
        let hit = frame_hit(
            x,
            y,
            allocation.width() as f64,
            allocation.height() as f64,
            window_for_frame.is_maximized(),
        );

        match hit {
            FrameHit::Resize(edge) => {
                consume_release_for_press.set(true);
                pending_drag_for_press.borrow_mut().take();
                window_for_frame.begin_resize_drag(
                    edge,
                    event.button() as i32,
                    root_x.round() as i32,
                    root_y.round() as i32,
                    event.time(),
                );
                gtk::glib::Propagation::Stop
            }
            FrameHit::Drag => {
                consume_release_for_press.set(true);
                if event.event_type() == gtk::gdk::EventType::DoubleButtonPress {
                    pending_drag_for_press.borrow_mut().take();
                    if window_for_frame.is_maximized() {
                        window_for_frame.unmaximize();
                    } else {
                        window_for_frame.maximize();
                    }
                } else {
                    pending_drag_for_press.replace(Some(PendingDrag {
                        local_x: x,
                        local_y: y,
                        root_x: root_x.round() as i32,
                        root_y: root_y.round() as i32,
                        time: event.time(),
                    }));
                }
                gtk::glib::Propagation::Stop
            }
            FrameHit::Client => {
                consume_release_for_press.set(false);
                pending_drag_for_press.borrow_mut().take();
                gtk::glib::Propagation::Proceed
            }
        }
    });

    let window_for_motion = window.clone();
    let pending_drag_for_motion = pending_drag.clone();
    webview.connect_motion_notify_event(move |_widget, event| {
        if !event
            .state()
            .contains(gtk::gdk::ModifierType::BUTTON1_MASK)
        {
            pending_drag_for_motion.borrow_mut().take();
            return gtk::glib::Propagation::Proceed;
        }

        let (x, y) = event.position();
        let should_start = pending_drag_for_motion
            .borrow()
            .as_ref()
            .map(|pending| drag_threshold_exceeded(pending.local_x, pending.local_y, x, y))
            .unwrap_or(false);
        if !should_start {
            return gtk::glib::Propagation::Proceed;
        }

        if let Some(pending) = pending_drag_for_motion.borrow_mut().take() {
            window_for_motion.begin_move_drag(
                1,
                pending.root_x,
                pending.root_y,
                pending.time,
            );
        }
        gtk::glib::Propagation::Stop
    });

    webview.connect_button_release_event(move |_widget, event| {
        if event.button() == 1 && consume_release.replace(false) {
            pending_drag.borrow_mut().take();
            gtk::glib::Propagation::Stop
        } else {
            gtk::glib::Propagation::Proceed
        }
    });

    vbox.pack_start(&webview, true, true, 0);

    // ====== 加载前端 ======
    let frontend_url = frontend_url();
    info!("加载前端: {}", frontend_url);

    // 注入 IPC 桥脚本（在文档加载前注入，确保 window.ipc 和 window.__lotus 在前端 JS 执行前就绪）
    // 关键：__lotus 带消息缓冲队列，这样即使 Rust 在 app.js 加载前就发消息，也不会丢
    // 同时注入 home 路径，供前端 shortenPath 用
    let home_path = std::env::var("HOME").unwrap_or_default();
    let ipc_bridge = format!(r#"
        // 0. 注入 home 路径（供前端路径简化显示用）
        if (!window.__lotus) {{
            window.__lotus = {{ home: {home_json} }};
        }} else {{
            window.__lotus.home = {home_json};
        }}
        // 1. window.ipc.postMessage 桥（前端 → Rust）
        if (!window.ipc) {{
            Object.defineProperty(window, 'ipc', {{
                value: Object.freeze({{
                    postMessage: function(x) {{
                        window.webkit.messageHandlers['ipc'].postMessage(x);
                    }}
                }}),
                configurable: false
            }});
        }}
        // 2. window.__lotus.onMessage 桥（Rust → 前端），带缓冲队列
        //    handleServerMessage 由 app.js 后续定义；ready 前的消息入队
        window.__lotus._queue = window.__lotus._queue || [];
        window.__lotus._ready = window.__lotus._ready || false;
        window.__lotus._handle = window.__lotus._handle || null;
        window.__lotus.onMessage = function(msg) {{
            if (window.__lotus._handle) {{
                window.__lotus._handle(msg);
            }} else {{
                window.__lotus._queue.push(msg);
            }}
        }};
    "#, home_json = serde_json::to_string(&home_path).unwrap_or_else(|_| "\"\"".into()));
    let user_script = UserScript::new(
        &ipc_bridge,
        UserContentInjectedFrames::TopFrame,
        UserScriptInjectionTime::Start,
        &[],
        &[],
    );
    content_manager.add_script(&user_script);

    // 加载前端 URL
    webview.load_uri(&frontend_url);

    // ====== PTY 输出 → 前端 的定时批量推送 ======
    // 用 GLib timeout（每 16ms 一次，~60fps）轮询 TermEvent channel
    let state_for_tick = state.clone();
    let webview_for_tick = webview.clone();
    gtk::glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
        let mut to_send: Vec<ServerMessage> = Vec::new();
        let mut exited_tabs: Vec<u32> = Vec::new();
        {
            let mut s = state_for_tick.lock().unwrap();
            // 非阻塞拉取所有 TermEvent（先收集，避免可变借用冲突）
            let mut command_starts: Vec<(u32, String, String)> = Vec::new();
            let mut command_runs: Vec<(u32, String, String, i32)> = Vec::new();
            if let Some(rx) = s.event_rx.as_mut() {
                while let Ok(event) = rx.try_recv() {
                    match event {
                        TermEvent::Output { tab_id, data } => {
                            let b64 = ipc::encode_b64(&data);
                            to_send.push(ServerMessage::Output { tab_id, data: b64 });
                        }
                        TermEvent::Exited { tab_id } => {
                            exited_tabs.push(tab_id);
                            to_send.push(ServerMessage::TabClosed { tab_id });
                        }
                        TermEvent::CommandStart { tab_id, cmd, cwd } => {
                            command_starts.push((tab_id, cmd, cwd));
                        }
                        TermEvent::CommandRun { tab_id, cmd, cwd, code } => {
                            // 先收集，循环外处理（避免和 rx 的借用冲突）
                            command_runs.push((tab_id, cmd, cwd, code));
                        }
                    }
                }
            }
            // 命令开始 → 记录时间 + 通知前端（tab 忙碌徽章）
            let mut session_dirty = false;
            for (tab_id, cmd, cwd) in command_starts {
                if cmd.trim().is_empty() {
                    continue;
                }
                if let Some(m) = s.manager.as_mut() {
                    m.set_cwd(tab_id, cwd.clone());
                }
                s.running_cmds
                    .insert(tab_id, (cmd.clone(), Instant::now()));
                to_send.push(ServerMessage::CommandStarted { tab_id, cmd, cwd });
                session_dirty = true;
            }
            // 处理捕获到的命令（这时 rx 借用已结束）
            for (tab_id, cmd, cwd, code) in command_runs {
                // 跳过空命令（bash 启动时的空 history）
                if cmd.trim().is_empty() {
                    continue;
                }
                if let Some(m) = s.manager.as_mut() {
                    m.set_cwd(tab_id, cwd.clone());
                }
                session_dirty = true;
                let duration_ms = s
                    .running_cmds
                    .remove(&tab_id)
                    .map(|(_, t0)| t0.elapsed().as_millis() as u64)
                    .unwrap_or(0);
                let entry = HistoryEntry {
                    cmd: cmd.clone(),
                    cwd: cwd.clone(),
                    ts: now_ts(),
                    code,
                };
                s.history.append(entry.clone());
                // 持久化（每次 append 都保存，1000 条规模开销可接受）
                if let Err(e) = s.history.save() {
                    warn!("历史保存失败：{}", e);
                }
                // 更新最近目录缓存
                let new_recents = s.history.recent_dirs();
                let recents_changed = new_recents != s.recents;
                if recents_changed {
                    s.recents = new_recents.clone();
                    to_send.push(ServerMessage::RecentsList { paths: new_recents });
                }
                // 命令结束事件（前端：清徽章 / 派发队列 / 按需通知）
                to_send.push(ServerMessage::CommandFinished {
                    tab_id,
                    cmd: cmd.clone(),
                    cwd: cwd.clone(),
                    code,
                    duration_ms,
                });
                // 推送单条历史给前端（如果用户当前在历史面板会实时更新）
                to_send.push(ServerMessage::HistoryAppended {
                    entry: HistoryEntryPayload {
                        cmd: entry.cmd,
                        cwd: entry.cwd,
                        ts: entry.ts,
                        code: entry.code,
                    },
                });
            }
            // 处理已退出的 tab（这时 rx 借用已结束）
            for tab_id in &exited_tabs {
                s.running_cmds.remove(tab_id);
                // 清理 active 映射
                s.active_tab_by_project.retain(|_, tid| *tid != *tab_id);
                if let Some(m) = s.manager.as_mut() {
                    m.close_tab(*tab_id);
                }
                session_dirty = true;
            }
            if session_dirty {
                persist_sessions(&s);
            }
            // 加上 pending_outputs（IPC handler 累积的）
            if !s.pending_outputs.is_empty() {
                to_send.append(&mut s.pending_outputs);
            }
        }
        // 下发给前端
        for msg in to_send {
            let json = msg.to_json();
            // 直接调用 window.__lotus.onMessage（__lotus 在 app.js 最早定义，带缓冲队列）
            // 把 JSON 对象字面量直接作为 JS 传入；onMessage 内部会处理
            let js = format!(
                "if (window.__lotus && window.__lotus.onMessage) {{ window.__lotus.onMessage({}); }}",
                json
            );
            // run_javascript 异步执行，回调里打印错误便于诊断
            // webkit2gtk 2.40+ 推荐 evaluate_javascript，此处 API 仍可用
            #[allow(deprecated)]
            {
                let js_for_log = js.chars().take(120).collect::<String>();
                webview_for_tick.run_javascript(
                    &js,
                    None::<&gtk::gio::Cancellable>,
                    move |result: Result<JavascriptResult, gtk::glib::Error>| {
                        if let Err(e) = result {
                            log::warn!("run_javascript 失败 [{}...]: {}", js_for_log, e);
                        }
                    },
                );
            }
        }
        gtk::glib::ControlFlow::Continue
    });

    // 显示所有 widget，进入 GTK 主循环
    window.show_all();
    info!("进入 GTK 主循环");
    gtk::main();

    // 退出清理：先落盘 tab 会话，再杀 PTY
    let mut s = state.lock().unwrap();
    persist_sessions(&s);
    if let Some(m) = s.manager.as_mut() {
        m.close_all();
    }
    info!("Lotus 退出");
    Ok(())
}

/// 处理前端发来的消息（JS → Rust）
fn handle_client_message(body: &str, state: &Arc<Mutex<AppState>>, window: &gtk::Window) {
    let msg: ClientMessage = match serde_json::from_str(body) {
        Ok(m) => m,
        Err(e) => {
            warn!("解析前端消息失败: {} | body: {}", e, body);
            return;
        }
    };

    match &msg {
        ClientMessage::WindowMinimize => {
            window.iconify();
            return;
        }
        ClientMessage::WindowToggleMaximize => {
            if window.is_maximized() {
                window.unmaximize();
            } else {
                window.maximize();
            }
            return;
        }
        _ => {}
    }

    let mut s = state.lock().unwrap();
    match msg {
        ClientMessage::Ready => {
            info!("前端已就绪");
            let theme_payload = theme_to_payload(&s.theme);
            s.pending_outputs.push(ServerMessage::Theme {
                theme: theme_payload,
            });
            if !s.first_tab_created {
                s.first_tab_created = true;
                // Restore Tab metadata only. Commands are never replayed.
                restore_sessions(&mut s);
            }
        }
        ClientMessage::Input { tab_id, data } => match ipc::decode_b64(&data) {
            Ok(bytes) => {
                if let Some(m) = s.manager.as_mut() {
                    if let Err(e) = m.write(tab_id, &bytes) {
                        error!("写入 tab {} 失败: {}", tab_id, e);
                    }
                }
            }
            Err(e) => warn!("base64 解码失败: {}", e),
        },
        ClientMessage::NewTab { cwd: _ } => {
            let pid = s.current_project_id;
            if let Some(m) = s.manager.as_mut() {
                match m.create_tab(80, 24, None, pid) {
                    Ok(tab_id) => {
                        info!("新建 tab: {}", tab_id);
                        let title = format!("lotus {}", tab_id);
                        m.set_title(tab_id, title.clone());
                        s.active_tab_by_project.insert(pid, tab_id);
                        s.pending_outputs.push(ServerMessage::TabCreated {
                            tab_id,
                            title,
                            cols: 80,
                            rows: 24,
                            project_id: pid,
                            activate: true,
                        });
                        persist_sessions(&s);
                    }
                    Err(e) => error!("新建 tab 失败: {}", e),
                }
            }
        }
        ClientMessage::CloseTab { tab_id } => {
            // 记录所属项目，便于关闭后选下一个 active
            let proj = s
                .manager
                .as_ref()
                .and_then(|m| m.list_tabs().into_iter().find(|t| t.tab_id == tab_id))
                .map(|t| t.project_id);
            if let Some(m) = s.manager.as_mut() {
                m.close_tab(tab_id);
            }
            s.running_cmds.remove(&tab_id);
            if let Some(pid) = proj {
                if s.active_tab_by_project.get(&pid) == Some(&tab_id) {
                    // 选同项目剩余第一个
                    let next = s.manager.as_ref().and_then(|m| {
                        m.list_tabs()
                            .into_iter()
                            .find(|t| t.project_id == pid)
                            .map(|t| t.tab_id)
                    });
                    match next {
                        Some(nid) => {
                            s.active_tab_by_project.insert(pid, nid);
                        }
                        None => {
                            s.active_tab_by_project.remove(&pid);
                        }
                    }
                }
            } else {
                s.active_tab_by_project.retain(|_, tid| *tid != tab_id);
            }
            s.pending_outputs.push(ServerMessage::TabClosed { tab_id });
            persist_sessions(&s);
        }
        ClientMessage::SwitchTab { tab_id } => {
            if let Some(pid) = s.manager.as_ref().and_then(|m| {
                m.list_tabs()
                    .into_iter()
                    .find(|t| t.tab_id == tab_id)
                    .map(|t| t.project_id)
            }) {
                s.active_tab_by_project.insert(pid, tab_id);
                persist_sessions(&s);
            }
        }
        ClientMessage::Resize { tab_id, cols, rows } => {
            if let Some(m) = s.manager.as_mut() {
                if let Err(e) = m.resize(tab_id, cols, rows) {
                    warn!("resize tab {} 失败: {}", tab_id, e);
                }
            }
        }
        ClientMessage::WindowMinimize | ClientMessage::WindowToggleMaximize => unreachable!(),
        // ===== 设置相关 =====
        ClientMessage::Devtools => {
            info!("请求打开 devtools");
            // Devtools 由前端的 WebInspector 处理，这里只记录；前端可用右键菜单"检查"打开
            // 真正的 inspector 打开需要持久的 webview 引用，留到后续优化
        }
        ClientMessage::GetConfig => {
            info!("前端请求配置");
            // 推送当前配置 + 主题列表 + 字体列表 + shell 列表
            let cfg_payload = config_to_payload(&s.config);
            s.pending_outputs.push(ServerMessage::Config { config: cfg_payload });
            s.pending_outputs.push(ServerMessage::ThemesList {
                names: Theme::list().iter().map(|s| s.to_string()).collect(),
            });
            s.pending_outputs.push(ServerMessage::FontsList {
                names: list_mono_fonts(),
            });
            s.pending_outputs.push(ServerMessage::ShellsList {
                names: list_shells(),
            });
        }
        ClientMessage::SetTheme { name } => {
            info!("实时切换主题预览: {}", name);
            let theme = Theme::by_name(&name);
            s.theme = theme.clone();
            s.config.theme = name;
            s.pending_outputs.push(ServerMessage::Theme {
                theme: theme_to_payload(&theme),
            });
        }
        ClientMessage::SaveConfig { config } => {
            info!("保存配置: {:?}", config);
            // 更新内存中的配置
            s.config.theme = config.theme.clone();
            s.config.font = config.font.clone();
            s.config.font_size = config.font_size;
            s.config.opacity = config.opacity;
            // shell：空字符串当作 None（用默认）
            s.config.shell = if config.shell.is_empty() {
                None
            } else {
                Some(config.shell.clone())
            };
            // 写盘
            let save_result = s.config.save();
            s.pending_outputs.push(ServerMessage::ConfigSaved {
                ok: save_result.is_ok(),
            });
            if let Err(e) = save_result {
                error!("保存配置失败: {}", e);
            }
        }
        // ===== 历史 / 书签 =====
        ClientMessage::GetHistory { query } => {
            let q = query.unwrap_or_default();
            let entries: Vec<HistoryEntryPayload> = s
                .history
                .search(&q)
                .into_iter()
                .map(|e| HistoryEntryPayload {
                    cmd: e.cmd.clone(),
                    cwd: e.cwd.clone(),
                    ts: e.ts,
                    code: e.code,
                })
                .collect();
            s.pending_outputs.push(ServerMessage::HistoryList { entries });
            // 顺便推一下最近目录（侧边栏用）
            let recents = s.recents.clone();
            s.pending_outputs.push(ServerMessage::RecentsList { paths: recents });
        }
        ClientMessage::ClearHistory => {
            s.history.clear();
            let _ = s.history.save();
            s.recents.clear();
            s.pending_outputs.push(ServerMessage::HistoryList { entries: vec![] });
            s.pending_outputs.push(ServerMessage::RecentsList { paths: vec![] });
            info!("历史已清空");
        }
        ClientMessage::GetBookmarks => {
            let entries: Vec<BookmarkEntryPayload> = s
                .bookmarks
                .entries
                .iter()
                .map(|b| BookmarkEntryPayload {
                    id: b.id,
                    name: b.name.clone(),
                    path: b.path.clone(),
                })
                .collect();
            s.pending_outputs.push(ServerMessage::BookmarksList { entries });
        }
        ClientMessage::AddBookmark { name, path } => {
            let id = s.bookmarks.add(name, path);
            if let Err(e) = s.bookmarks.save() {
                warn!("书签保存失败：{}", e);
            }
            // 找到刚加的那条，clone 出来避免借用冲突
            let added = s.bookmarks.entries.iter().find(|b| b.id == id).map(|b| {
                BookmarkEntryPayload {
                    id: b.id,
                    name: b.name.clone(),
                    path: b.path.clone(),
                }
            });
            if let Some(entry) = added {
                s.pending_outputs.push(ServerMessage::BookmarkAdded { entry });
            }
        }
        ClientMessage::RemoveBookmark { id } => {
            if s.bookmarks.remove(id) {
                let _ = s.bookmarks.save();
                s.pending_outputs.push(ServerMessage::BookmarkRemoved { id });
            }
        }
        ClientMessage::RunInTab { tab_id, command } => {
            // 把命令 + 换行写入 PTY（等效于用户手敲）
            if let Some(m) = s.manager.as_mut() {
                let mut line = command;
                line.push('\n');
                if let Err(e) = m.write(tab_id, line.as_bytes()) {
                    warn!("RunInTab 写入失败：{}", e);
                }
            }
        }
        // ===== 项目 =====
        ClientMessage::GetProjects => {
            let entries: Vec<ProjectPayload> = s
                .projects
                .list()
                .into_iter()
                .map(|m| ProjectPayload {
                    id: m.id,
                    name: m.name.clone(),
                    cwd: m.cwd.clone(),
                })
                .collect();
            // 当前项目信息从 current_project_id + metas 获取
            let cur = s.projects.get(s.current_project_id).cloned();
            s.pending_outputs.push(ServerMessage::ProjectsList { entries });
            if let Some(m) = cur {
                s.pending_outputs.push(ServerMessage::ProjectSwitched {
                    id: m.id,
                    name: m.name.clone(),
                    cwd: m.cwd.clone(),
                });
            }
        }
        ClientMessage::NewProject { name, cwd } => {
            let id = s.projects.create(name, expand_user_path(&cwd));
            let _ = s.projects.save_metadata();
            info!("新建项目 id={}", id);
            // 自动切换到新项目
            switch_project_in_state(&mut s, id);
        }
        ClientMessage::SwitchProject { id } => {
            switch_project_in_state(&mut s, id);
        }
        ClientMessage::DeleteProject { id } => {
            if s.projects.delete(id) {
                info!("删除项目 id={}", id);
                // 关掉该项目下所有 tab，并清 active
                let to_close: Vec<u32> = s
                    .manager
                    .as_ref()
                    .map(|m| {
                        m.list_tabs()
                            .into_iter()
                            .filter(|t| t.project_id == id)
                            .map(|t| t.tab_id)
                            .collect()
                    })
                    .unwrap_or_default();
                for tid in to_close {
                    if let Some(m) = s.manager.as_mut() {
                        m.close_tab(tid);
                    }
                    s.running_cmds.remove(&tid);
                    s.pending_outputs.push(ServerMessage::TabClosed { tab_id: tid });
                }
                s.active_tab_by_project.remove(&id);
                // 如果删的是当前项目，切到第一个剩余项目
                if s.current_project_id == id {
                    let first_id = s.projects.list().first().map(|m| m.id);
                    if let Some(fid) = first_id {
                        switch_project_in_state(&mut s, fid);
                    }
                }
                // 推送更新后的项目列表
                let entries: Vec<ProjectPayload> = s
                    .projects
                    .list()
                    .into_iter()
                    .map(|m| ProjectPayload {
                        id: m.id,
                        name: m.name.clone(),
                        cwd: m.cwd.clone(),
                    })
                    .collect();
                s.pending_outputs.push(ServerMessage::ProjectsList { entries });
                persist_sessions(&s);
            }
        }
        ClientMessage::RenameProject { id, name } => {
            if s.projects.rename(id, name) {
                info!("重命名项目 id={}", id);
                let entries: Vec<ProjectPayload> = s
                    .projects
                    .list()
                    .into_iter()
                    .map(|m| ProjectPayload {
                        id: m.id,
                        name: m.name.clone(),
                        cwd: m.cwd.clone(),
                    })
                    .collect();
                s.pending_outputs.push(ServerMessage::ProjectsList { entries });
                // 如果是当前项目，也推 ProjectSwitched 更新标题
                if id == s.current_project_id {
                    let switched = s.projects.get(id).map(|m| ServerMessage::ProjectSwitched {
                        id,
                        name: m.name.clone(),
                        cwd: m.cwd.clone(),
                    });
                    if let Some(msg) = switched {
                        s.pending_outputs.push(msg);
                    }
                }
            }
        }
        ClientMessage::ClipboardWrite { text } => {
            clipboard_set_text(&text);
        }
        ClientMessage::PasteToTab { tab_id } => {
            if let Some(text) = clipboard_get_text() {
                if let Some(m) = s.manager.as_mut() {
                    if let Err(e) = m.write(tab_id, text.as_bytes()) {
                        error!("粘贴到 tab {} 失败: {}", tab_id, e);
                    }
                }
            } else {
                warn!("系统剪贴板为空或不可读");
            }
        }
        ClientMessage::ClipboardRead { request_id } => {
            let text = clipboard_get_text().unwrap_or_default();
            s.pending_outputs.push(ServerMessage::ClipboardText { request_id, text });
        }
        ClientMessage::GetAgents => {
            s.pending_outputs.push(ServerMessage::AgentsList {
                agents: detect_agents(),
            });
        }
        ClientMessage::LaunchAgent {
            command,
            title,
            cwd,
        } => {
            let pid = s.current_project_id;
            let run_cwd = cwd
                .map(|c| expand_user_path(&c))
                .or_else(|| {
                    s.projects
                        .get(s.current_project_id)
                        .map(|m| expand_user_path(&m.cwd))
                });
            let tab_title = title.unwrap_or_else(|| {
                command
                    .split_whitespace()
                    .next()
                    .unwrap_or("agent")
                    .to_string()
            });
            if let Some(m) = s.manager.as_mut() {
                match m.create_tab(80, 24, run_cwd.as_deref(), pid) {
                    Ok(tab_id) => {
                        info!("启动 agent tab {} → {}", tab_id, command);
                        m.set_title(tab_id, tab_title.clone());
                        s.active_tab_by_project.insert(pid, tab_id);
                        s.pending_outputs.push(ServerMessage::TabCreated {
                            tab_id,
                            title: tab_title.clone(),
                            cols: 80,
                            rows: 24,
                            project_id: pid,
                            activate: true,
                        });
                        persist_sessions(&s);
                        // shell 初始化需要一点时间，延迟写入启动命令
                        drop(s);
                        schedule_tab_commands(
                            state,
                            vec![(tab_id, command, tab_title)],
                        );
                        return;
                    }
                    Err(e) => error!("LaunchAgent 创建 tab 失败: {}", e),
                }
            }
        }
        ClientMessage::DesktopNotify { title, body } => {
            desktop_notify(&title, &body);
        }
        ClientMessage::Quit => {
            info!("前端请求退出");
            gtk::main_quit();
        }
    }
}

/// 读取 GTK 系统剪贴板文本
fn clipboard_get_text() -> Option<String> {
    let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
    clipboard
        .wait_for_text()
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
}

/// 写入 GTK 系统剪贴板文本
fn clipboard_set_text(text: &str) {
    let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
    clipboard.set_text(text);
    // 同步到 X11 的 CLIPBOARD 所有者，避免应用退出前丢内容
    clipboard.store();
}

/// 展开 `~` / `~/...` 为 $HOME 绝对路径
fn expand_user_path(path: &str) -> String {
    let p = path.trim();
    if p == "~" {
        return std::env::var("HOME").unwrap_or_else(|_| p.to_string());
    }
    if let Some(rest) = p.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            return format!("{home}/{rest}");
        }
    }
    p.to_string()
}

/// 桌面通知（优先 notify-send，失败则静默）
fn desktop_notify(title: &str, body: &str) {
    // 尝试 gdbus / notify-send，不阻塞主循环
    let title = title.to_string();
    let body = body.to_string();
    std::thread::spawn(move || {
        let ok = std::process::Command::new("notify-send")
            .args([
                "--app-name=Lotus",
                "--icon=utilities-terminal",
                "--expire-time=8000",
                &title,
                &body,
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            log::debug!("notify-send 不可用，跳过桌面通知");
        }
    });
}

/// 探测常见 agent CLI 是否已安装
fn detect_agents() -> Vec<AgentInfoPayload> {
    // (id, name, bin, default_cmd, icon)
    const CATALOG: &[(&str, &str, &str, &str, &str)] = &[
        ("claude", "Claude Code", "claude", "claude", "✦"),
        ("codex", "OpenAI Codex", "codex", "codex", "◎"),
        ("opencode", "OpenCode", "opencode", "opencode", "⌘"),
        ("gemini", "Gemini CLI", "gemini", "gemini", "◆"),
        ("aider", "Aider", "aider", "aider", "✎"),
        ("pi", "Pi Agent", "pi", "pi", "π"),
        ("cursor-agent", "Cursor Agent", "cursor-agent", "cursor-agent", "⌁"),
        ("continue", "Continue", "cn", "cn", "▶"),
    ];

    CATALOG
        .iter()
        .map(|(id, name, bin, cmd, icon)| {
            let path = which_bin(bin);
            AgentInfoPayload {
                id: (*id).to_string(),
                name: (*name).to_string(),
                bin: (*bin).to_string(),
                cmd: (*cmd).to_string(),
                icon: (*icon).to_string(),
                installed: path.is_some(),
                path,
            }
        })
        .collect()
}

fn which_bin(bin: &str) -> Option<String> {
    // 1) 直接 which
    if let Ok(out) = std::process::Command::new("which").arg(bin).output() {
        if out.status.success() {
            let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !p.is_empty() {
                return Some(p);
            }
        }
    }
    // 2) 常见用户路径
    if let Ok(home) = std::env::var("HOME") {
        let candidates = [
            format!("{home}/.local/bin/{bin}"),
            format!("{home}/.cargo/bin/{bin}"),
            format!("{home}/go/bin/{bin}"),
            format!("/usr/local/bin/{bin}"),
        ];
        for c in candidates {
            if std::path::Path::new(&c).is_file() {
                return Some(c);
            }
        }
    }
    None
}

/// 把当前全部 tab 元数据写入 sessions.json（重启后恢复用）
fn persist_sessions(s: &AppState) {
    let Some(m) = s.manager.as_ref() else {
        return;
    };
    let infos = m.list_tabs();
    let runtime: Vec<(u32, u32)> = infos.iter().map(|t| (t.tab_id, t.project_id)).collect();
    let tabs: Vec<TabSession> = infos
        .into_iter()
        .map(|t| TabSession {
            project_id: t.project_id,
            title: t.title,
            cwd: t.cwd,
            command: None,
        })
        .collect();
    let mut store = SessionStore::default();
    store.replace_from(tabs, &s.active_tab_by_project, &runtime);
    if let Err(e) = store.save() {
        warn!("保存 tab 会话失败：{}", e);
    }
}

/// 启动时从 sessions.json 恢复各项目 tab。
fn restore_sessions(s: &mut AppState) {
    let mut store = SessionStore::load();
    store.discard_commands();
    let active_by_project = store.active_by_project.clone();
    // 只恢复仍然存在的项目
    let mut pending: Vec<TabSession> = store
        .tabs
        .into_iter()
        .filter(|t| s.projects.get(t.project_id).is_some())
        .collect();

    // 若当前项目一条都没有，补一个默认 tab 描述
    let cur = s.current_project_id;
    if !pending.iter().any(|t| t.project_id == cur) {
        let cwd = s
            .projects
            .get(cur)
            .map(|m| expand_user_path(&m.cwd))
            .unwrap_or_else(|| {
                std::env::current_dir()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|_| "/".into())
            });
        pending.push(TabSession {
            project_id: cur,
            title: "lotus".into(),
            cwd,
            command: None,
        });
    }

    // 计算每个项目的 active 下标
    let mut idx_in_proj: HashMap<u32, usize> = HashMap::new();

    info!("恢复 tab 会话：共 {} 个", pending.len());

    for sess in pending {
        let pid = sess.project_id;
        let local_idx = *idx_in_proj.entry(pid).or_insert(0);
        *idx_in_proj.get_mut(&pid).unwrap() += 1;

        let active_idx = active_by_project.get(&pid).copied().unwrap_or(0);
        // 该项目无记录时默认激活第 0 个；否则按记录的下标
        let activate = local_idx == active_idx
            || (local_idx == 0 && !active_by_project.contains_key(&pid));

        let cwd = expand_user_path(&sess.cwd);
        let title = if sess.title.trim().is_empty() {
            "lotus".to_string()
        } else {
            sess.title.clone()
        };

        let created = s.manager.as_mut().and_then(|m| {
            match m.create_tab(80, 24, Some(&cwd), pid) {
                Ok(tab_id) => {
                    m.set_title(tab_id, title.clone());
                    Some(tab_id)
                }
                Err(e) => {
                    error!("恢复 tab 失败 (project={}, cwd={}): {}", pid, cwd, e);
                    None
                }
            }
        });

        if let Some(tab_id) = created {
            if activate {
                s.active_tab_by_project.insert(pid, tab_id);
            }
            s.pending_outputs.push(ServerMessage::TabCreated {
                tab_id,
                title: title.clone(),
                cols: 80,
                rows: 24,
                project_id: pid,
                activate,
            });
        }
    }

    // 若恢复过程中当前项目仍无 tab（全失败），兜底开一个
    let has_cur = s
        .manager
        .as_ref()
        .map(|m| m.has_tabs_for_project(cur))
        .unwrap_or(false);
    if !has_cur {
        if let Some(m) = s.manager.as_mut() {
            match m.create_tab(80, 24, None, cur) {
                Ok(tab_id) => {
                    m.set_title(tab_id, "lotus".into());
                    s.active_tab_by_project.insert(cur, tab_id);
                    s.pending_outputs.push(ServerMessage::TabCreated {
                        tab_id,
                        title: "lotus".into(),
                        cols: 80,
                        rows: 24,
                        project_id: cur,
                        activate: true,
                    });
                }
                Err(e) => error!("兜底创建 tab 失败: {}", e),
            }
        }
    }

    persist_sessions(s);
}

/// 延迟把命令写入新 tab 的 PTY（等 shell 初始化）
fn schedule_tab_commands(state: &Arc<Mutex<AppState>>, items: Vec<(u32, String, String)>) {
    if items.is_empty() {
        return;
    }
    let state2 = Arc::clone(state);
    gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(400), move || {
        let mut st = state2.lock().unwrap();
        for (tab_id, cmd, title) in items {
            if let Some(m) = st.manager.as_mut() {
                let mut line = cmd;
                line.push('\n');
                if let Err(e) = m.write(tab_id, line.as_bytes()) {
                    warn!("恢复/启动命令写入 tab {} 失败: {}", tab_id, e);
                } else {
                    st.running_cmds
                        .insert(tab_id, (line.trim().to_string(), Instant::now()));
                    st.pending_outputs.push(ServerMessage::CommandStarted {
                        tab_id,
                        cmd: line.trim().to_string(),
                        cwd: String::new(),
                    });
                    st.pending_outputs.push(ServerMessage::TitleChanged {
                        tab_id,
                        title,
                    });
                }
            }
        }
    });
}

/// 切换项目：持久化旧项目历史/书签 → 不杀 tab → 加载新项目数据 → 必要时开新 tab
fn switch_project_in_state(s: &mut AppState, new_id: u32) {
    // 目标项目必须存在
    let new_meta = match s.projects.get(new_id).cloned() {
        Some(m) => m,
        None => {
            warn!("切换失败：项目 {} 不存在", new_id);
            return;
        }
    };

    info!("切换项目：{} → {} ({})", s.current_project_id, new_id, new_meta.name);

    // 1. 持久化当前项目的快照（history/bookmarks）回 ProjectStore 的数据文件
    if let Some(old_dir) = s.projects.project_dir(s.current_project_id) {
        let _ = std::fs::create_dir_all(&old_dir);
        if let Err(e) = s.history.save_in(&old_dir) {
            warn!("保存旧项目历史失败：{}", e);
        }
        if let Err(e) = s.bookmarks.save_in(&old_dir) {
            warn!("保存旧项目书签失败：{}", e);
        }
    }

    // 2. ⚠️ 方案 B：不杀任何 tab！只更新默认 cwd（后续新建 tab 落在新项目目录）
    //    原 tab 继续在后台运行，前端按 project_id 过滤显示。
    if let Some(m) = s.manager.as_mut() {
        m.set_default_cwd(expand_user_path(&new_meta.cwd));
    }

    // 3. 加载新项目数据到快照
    if let Some(proj) = s.projects.load_project(new_id) {
        s.history = proj.history;
        s.bookmarks = proj.bookmarks;
    } else {
        s.history = HistoryStore::default();
        s.bookmarks = BookmarkStore::default();
    }
    s.recents = s.history.recent_dirs();
    s.current_project_id = new_id;

    // 4. 更新 config 的 last_project_id 并持久化（下次启动恢复）
    s.config.last_project_id = Some(new_id);
    let _ = s.config.save();

    // 5. 仅当目标项目 0 个存活 tab 时，才创建首个 tab（避免空白界面）
    let need_new_tab = s
        .manager
        .as_ref()
        .map(|m| !m.has_tabs_for_project(new_id))
        .unwrap_or(true);
    if need_new_tab {
        if let Some(m) = s.manager.as_mut() {
            let expanded = expand_user_path(&new_meta.cwd);
            match m.create_tab(80, 24, Some(&expanded), new_id) {
                Ok(tab_id) => {
                    info!("新项目首个 tab: {}", tab_id);
                    m.set_title(tab_id, new_meta.name.clone());
                    s.active_tab_by_project.insert(new_id, tab_id);
                    s.pending_outputs.push(ServerMessage::TabCreated {
                        tab_id,
                        title: new_meta.name.clone(),
                        cols: 80,
                        rows: 24,
                        project_id: new_id,
                        activate: true,
                    });
                    persist_sessions(s);
                }
                Err(e) => error!("新项目创建 tab 失败：{}", e),
            }
        }
    }

    // 6. 推送项目切换通知 + 最近目录
    s.pending_outputs.push(ServerMessage::ProjectSwitched {
        id: new_id,
        name: new_meta.name.clone(),
        cwd: new_meta.cwd.clone(),
    });
    let recents = s.recents.clone();
    s.pending_outputs.push(ServerMessage::RecentsList { paths: recents });

    // 推送更新后的项目列表（高亮变化）
    let entries: Vec<ProjectPayload> = s
        .projects
        .list()
        .into_iter()
        .map(|m| ProjectPayload {
            id: m.id,
            name: m.name.clone(),
            cwd: m.cwd.clone(),
        })
        .collect();
    s.pending_outputs.push(ServerMessage::ProjectsList { entries });
}

/// 列出系统已装的常见等宽字体（用 fc-list 探测）
fn list_mono_fonts() -> Vec<String> {
    // 优先返回这些常见编程字体（按推荐度排序），用 fc-list 验证是否真的装了
    let candidates = [
        "JetBrains Mono",
        "Fira Code",
        "Cascadia Code",
        "Source Code Pro",
        "DejaVu Sans Mono",
        "Ubuntu Mono",
        "Ubuntu Sans Mono",
        "Liberation Mono",
        "Noto Mono",
        "Monaco",
        "Consolas",
        "Menlo",
    ];
    let installed = installed_font_names();
    let mut result: Vec<String> = candidates
        .iter()
        .filter(|c| installed.iter().any(|i| i == *c))
        .map(|s| s.to_string())
        .collect();
    // 去重保底
    result.dedup();
    if result.is_empty() {
        vec!["monospace".to_string()]
    } else {
        result
    }
}

/// 调 fc-list 获取已装字体名集合
fn installed_font_names() -> Vec<String> {
    let output = std::process::Command::new("fc-list")
        .arg(":spacing=mono")
        .arg("family")
        .output();
    match output {
        Ok(out) => String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
        Err(_) => Vec::new(),
    }
}

/// 列出系统可用的 shell
fn list_shells() -> Vec<String> {
    let candidates = ["/bin/bash", "/bin/zsh", "/usr/bin/fish", "/bin/sh"];
    candidates
        .iter()
        .filter(|c| std::path::Path::new(c).exists())
        .map(|s| s.to_string())
        .collect()
}

/// Config → ConfigPayload（给前端用）
fn config_to_payload(config: &Config) -> ConfigPayload {
    ConfigPayload {
        theme: config.theme.clone(),
        shell: config.shell.clone().unwrap_or_default(),
        font: config.font.clone(),
        font_size: config.font_size,
        opacity: config.opacity,
    }
}

/// 前端 URL（file:// 加载本地 index.html）
fn frontend_url() -> String {
    let path = frontend_index_path();
    format!("file://{}", path.to_string_lossy())
}

fn frontend_index_path() -> PathBuf {
    // 1) 环境变量覆盖（调试 / 自定义安装）
    if let Ok(p) = std::env::var("LOTUS_FRONTEND") {
        let p = PathBuf::from(p);
        if p.is_dir() {
            return p.join("index.html");
        }
        if p.is_file() {
            return p;
        }
    }

    let mut candidates: Vec<PathBuf> = Vec::new();

    // 2) 相对可执行文件：../share/lotus/frontend（deb 安装布局）
    if let Ok(exe) = std::env::current_exe() {
        if let Some(bin_dir) = exe.parent() {
            // /usr/bin/lotus → /usr/share/lotus/frontend
            candidates.push(
                bin_dir
                    .join("../share/lotus/frontend/index.html")
                    .canonicalize()
                    .unwrap_or_else(|_| bin_dir.join("../share/lotus/frontend/index.html")),
            );
            // 同目录旁 frontend/（便携解压布局）
            candidates.push(bin_dir.join("frontend/index.html"));
        }
    }

    // 3) 系统路径
    candidates.push(PathBuf::from("/usr/share/lotus/frontend/index.html"));
    candidates.push(PathBuf::from("/usr/local/share/lotus/frontend/index.html"));

    // 4) 开发路径
    candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("frontend/index.html"));
    candidates.push(PathBuf::from("./frontend/index.html"));

    for c in &candidates {
        if c.exists() {
            return c.clone();
        }
    }
    // 兜底：返回系统安装路径，便于日志排查
    PathBuf::from("/usr/share/lotus/frontend/index.html")
}

/// Theme → ThemePayload
fn theme_to_payload(theme: &Theme) -> ThemePayload {
    ThemePayload {
        name: theme.name.to_string(),
        bg: rgb_to_css(theme.bg),
        fg: rgb_to_css(theme.fg),
        accent: rgb_to_css(theme.accent),
        muted: rgb_to_css(theme.muted),
        success: rgb_to_css(theme.success),
        error: rgb_to_css(theme.error),
        block_border: rgb_to_css(theme.block_border),
        title_bg: rgb_to_css(theme.title_bg),
        sidebar_bg: rgb_to_css(theme.sidebar_bg),
        tab_bg: rgb_to_css(theme.tab_bg),
        is_dark: theme.is_dark,
    }
}

#[cfg(test)]
mod tests {
    use super::{FrameHit, drag_threshold_exceeded, frame_hit, preferred_gdk_backend};
    use gtk::gdk::WindowEdge;

    #[test]
    fn ukui_wayland_prefers_x11_for_frameless_window() {
        assert_eq!(
            preferred_gdk_backend(false, Some("wayland"), Some("UKUI"), Some(":0")),
            Some("x11")
        );
    }

    #[test]
    fn explicit_gdk_backend_is_preserved() {
        assert_eq!(
            preferred_gdk_backend(true, Some("wayland"), Some("UKUI"), Some(":0")),
            None
        );
    }

    #[test]
    fn x11_is_not_forced_outside_ukui_wayland_with_display() {
        let unsupported_environments = [
            (Some("x11"), Some("UKUI"), Some(":0")),
            (Some("wayland"), Some("GNOME"), Some(":0")),
            (Some("wayland"), Some("UKUI"), None),
            (Some("wayland"), Some("UKUI"), Some("")),
        ];

        for (session_type, desktop, display) in unsupported_environments {
            assert_eq!(
                preferred_gdk_backend(false, session_type, desktop, display),
                None
            );
        }

        assert_eq!(
            preferred_gdk_backend(
                false,
                Some("wayland"),
                Some("GNOME:UKUI"),
                Some(":0")
            ),
            Some("x11")
        );
    }

    #[test]
    fn frame_hit_detects_resize_edges_and_corners() {
        assert_eq!(
            frame_hit(1.0, 1.0, 1100.0, 720.0, false),
            FrameHit::Resize(WindowEdge::NorthWest)
        );
        assert_eq!(
            frame_hit(550.0, 1.0, 1100.0, 720.0, false),
            FrameHit::Resize(WindowEdge::North)
        );
        assert_eq!(
            frame_hit(1099.0, 719.0, 1100.0, 720.0, false),
            FrameHit::Resize(WindowEdge::SouthEast)
        );
        assert_eq!(
            frame_hit(1.0, 360.0, 1100.0, 720.0, false),
            FrameHit::Resize(WindowEdge::West)
        );
    }

    #[test]
    fn frame_hit_reserves_title_controls_and_supports_maximized_drag() {
        assert_eq!(frame_hit(200.0, 18.0, 1100.0, 720.0, false), FrameHit::Drag);
        assert_eq!(
            frame_hit(1050.0, 18.0, 1100.0, 720.0, false),
            FrameHit::Client
        );
        assert_eq!(frame_hit(550.0, 1.0, 1100.0, 720.0, true), FrameHit::Drag);
        assert_eq!(
            frame_hit(550.0, 200.0, 1100.0, 720.0, false),
            FrameHit::Client
        );
    }

    #[test]
    fn drag_requires_pointer_motion_past_threshold() {
        assert!(!drag_threshold_exceeded(10.0, 10.0, 13.0, 12.0));
        assert!(drag_threshold_exceeded(10.0, 10.0, 14.0, 10.0));
    }
}
