//! IPC 通信协议 —— Rust 后端与 JS 前端之间的消息格式
//!
//! - JS → Rust：通过 `window.ipc.postMessage(json)` 触发 webkit2gtk 的 script_message_received
//! - Rust → JS：通过 `webview.run_javascript("window.__lotus.onMessage({...})")`
//!
//! PTY 字节用 **base64** 编码传输（避免 JSON 转义二进制/控制字符的问题）。

use serde::{Deserialize, Serialize};

fn default_true() -> bool {
    true
}

/// 配置载荷（扁平结构，给设置面板读写用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigPayload {
    pub theme: String,
    pub shell: String,        // 解析后的 shell 路径（空字符串表示用默认）
    pub font: String,
    pub font_size: u16,
    pub opacity: f32,
    #[serde(default = "default_true", alias = "notifications_enabled")]
    pub agent_notifications_enabled: bool,
    #[serde(default)]
    pub command_notifications_enabled: bool,
}

/// 桌面通知类别，用于应用对应的用户偏好。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationKind {
    Agent,
    Command,
}

impl Default for NotificationKind {
    fn default() -> Self {
        Self::Command
    }
}

/// 历史记录载荷（给前端展示用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntryPayload {
    pub cmd: String,
    pub cwd: String,
    pub ts: u64,
    pub code: i32,
}

/// 书签载荷
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookmarkEntryPayload {
    pub id: u32,
    pub name: String,
    pub path: String,
}

/// 项目载荷
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectPayload {
    pub id: u32,
    pub name: String,
    pub cwd: String,
}

/// Agent CLI 探测结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInfoPayload {
    pub id: String,
    pub name: String,
    pub bin: String,
    pub cmd: String,
    pub icon: String,
    pub installed: bool,
    pub path: Option<String>,
}

/// JS → Rust 的消息
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    /// 键盘输入
    #[serde(rename = "input")]
    Input { tab_id: u32, data: String },
    /// 新建 tab
    #[serde(rename = "new_tab")]
    NewTab {
        #[serde(default)]
        cwd: Option<String>,
    },
    /// 关闭 tab
    #[serde(rename = "close_tab")]
    CloseTab { tab_id: u32 },
    /// 切换 tab
    #[serde(rename = "switch_tab")]
    SwitchTab { tab_id: u32 },
    /// 终端尺寸变化
    #[serde(rename = "resize")]
    Resize { tab_id: u32, cols: u16, rows: u16 },
    /// xterm.js 已完成一个输出块的解析，可继续发送该 tab 的下一块
    #[serde(rename = "output_ack")]
    OutputAck { tab_id: u32, seq: u64 },
    /// 应用启动完成通知
    #[serde(rename = "ready")]
    Ready,
    /// 退出应用
    #[serde(rename = "quit")]
    Quit,
    /// 最小化无边框窗口
    #[serde(rename = "window_minimize")]
    WindowMinimize,
    /// 在最大化和还原之间切换
    #[serde(rename = "window_toggle_maximize")]
    WindowToggleMaximize,
    /// 打开 devtools
    #[serde(rename = "devtools")]
    Devtools,
    // ===== 设置相关 =====
    /// 请求拉取当前配置 + 主题列表（打开设置面板时发）
    #[serde(rename = "get_config")]
    GetConfig,
    /// 保存配置到磁盘
    #[serde(rename = "save_config")]
    SaveConfig { config: ConfigPayload },
    /// 实时切换主题（预览，不写盘）
    #[serde(rename = "set_theme")]
    SetTheme { name: String },
    // ===== 历史 / 书签 =====
    /// 请求拉取历史列表（可选搜索词）
    #[serde(rename = "get_history")]
    GetHistory {
        #[serde(default)]
        query: Option<String>,
    },
    /// 清空历史
    #[serde(rename = "clear_history")]
    ClearHistory,
    /// 请求拉取书签列表
    #[serde(rename = "get_bookmarks")]
    GetBookmarks,
    /// 添加书签
    #[serde(rename = "add_bookmark")]
    AddBookmark { name: String, path: String },
    /// 删除书签
    #[serde(rename = "remove_bookmark")]
    RemoveBookmark { id: u32 },
    /// 在指定 tab 执行命令（点击历史/书签时用）
    #[serde(rename = "run_in_tab")]
    RunInTab { tab_id: u32, command: String },
    // ===== 项目 =====
    /// 请求拉取项目列表
    #[serde(rename = "get_projects")]
    GetProjects,
    /// 新建项目
    #[serde(rename = "new_project")]
    NewProject { name: String, cwd: String },
    /// 切换项目
    #[serde(rename = "switch_project")]
    SwitchProject { id: u32 },
    /// 删除项目
    #[serde(rename = "delete_project")]
    DeleteProject { id: u32 },
    /// 重命名项目
    #[serde(rename = "rename_project")]
    RenameProject { id: u32, name: String },
    // ===== 剪贴板（走 GTK 原生，WebKit clipboard API 在自定义协议下不可用）=====
    /// 写入系统剪贴板
    #[serde(rename = "clipboard_write")]
    ClipboardWrite { text: String },
    /// 读取系统剪贴板并粘贴到指定 tab 的 PTY
    #[serde(rename = "paste_to_tab")]
    PasteToTab { tab_id: u32 },
    /// 读取系统剪贴板并回传给前端（输入框等场景）
    #[serde(rename = "clipboard_read")]
    ClipboardRead {
        /// 前端用来匹配响应的请求 id
        request_id: u32,
    },
    // ===== Agent 工作流 =====
    /// 探测本机已安装的 agent CLI
    #[serde(rename = "get_agents")]
    GetAgents,
    /// 在新 tab 启动 agent（或任意命令）
    #[serde(rename = "launch_agent")]
    LaunchAgent {
        /// 要执行的命令（如 claude / codex）
        command: String,
        /// tab 标题
        #[serde(default)]
        title: Option<String>,
        /// 可选工作目录（默认当前项目 cwd）
        #[serde(default)]
        cwd: Option<String>,
    },
    /// 桌面通知（前端在命令完成/需要关注时请求）
    #[serde(rename = "desktop_notify")]
    DesktopNotify {
        #[serde(default)]
        kind: NotificationKind,
        #[serde(default)]
        tab_id: Option<u32>,
        title: String,
        body: String,
    },
}

/// Rust → JS 的消息
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    /// PTY 输出（base64 编码的字节）
    #[serde(rename = "output")]
    Output {
        tab_id: u32,
        seq: u64,
        data: String,
    },
    /// tab 已创建
    #[serde(rename = "tab_created")]
    TabCreated {
        tab_id: u32,
        title: String,
        cols: u16,
        rows: u16,
        project_id: u32,
        /// 是否立刻激活（批量恢复会话时仅 active tab 为 true）
        #[serde(default = "default_true")]
        activate: bool,
    },
    /// tab 已关闭
    #[serde(rename = "tab_closed")]
    TabClosed { tab_id: u32 },
    /// tab 标题更新
    #[serde(rename = "title_changed")]
    TitleChanged { tab_id: u32, title: String },
    /// 主题配置（启动时下发 / 切换主题时下发）
    #[serde(rename = "theme")]
    Theme { theme: ThemePayload },
    /// 配置数据（响应 GetConfig）
    #[serde(rename = "config")]
    Config { config: ConfigPayload },
    /// 可用主题列表
    #[serde(rename = "themes_list")]
    ThemesList { names: Vec<String> },
    /// 系统已装的等宽字体列表
    #[serde(rename = "fonts_list")]
    FontsList { names: Vec<String> },
    /// 系统可用的 shell 列表
    #[serde(rename = "shells_list")]
    ShellsList { names: Vec<String> },
    /// 配置保存结果
    #[serde(rename = "config_saved")]
    ConfigSaved { ok: bool },
    // ===== 历史 / 书签 =====
    /// 历史列表（响应 get_history）
    #[serde(rename = "history_list")]
    HistoryList { entries: Vec<HistoryEntryPayload> },
    /// 单条历史追加（实时推送）
    #[serde(rename = "history_appended")]
    HistoryAppended { entry: HistoryEntryPayload },
    /// 最近访问目录列表（侧边栏"最近"用）
    #[serde(rename = "recents_list")]
    RecentsList { paths: Vec<String> },
    /// 书签列表
    #[serde(rename = "bookmarks_list")]
    BookmarksList { entries: Vec<BookmarkEntryPayload> },
    /// 书签已添加
    #[serde(rename = "bookmark_added")]
    BookmarkAdded { entry: BookmarkEntryPayload },
    /// 书签已删除
    #[serde(rename = "bookmark_removed")]
    BookmarkRemoved { id: u32 },
    // ===== 项目 =====
    /// 项目列表
    #[serde(rename = "projects_list")]
    ProjectsList { entries: Vec<ProjectPayload> },
    /// 当前项目已切换（含新项目的 cwd）
    #[serde(rename = "project_switched")]
    ProjectSwitched { id: u32, name: String, cwd: String },
    /// 剪贴板文本（响应 clipboard_read）
    #[serde(rename = "clipboard_text")]
    ClipboardText { request_id: u32, text: String },
    // ===== Agent / 命令状态 =====
    /// 本机 agent CLI 列表
    #[serde(rename = "agents_list")]
    AgentsList { agents: Vec<AgentInfoPayload> },
    /// 命令开始（用于 tab 忙碌徽章）
    #[serde(rename = "command_started")]
    CommandStarted {
        tab_id: u32,
        cmd: String,
        cwd: String,
    },
    /// 命令结束（用于徽章清除、队列派发、通知）
    #[serde(rename = "command_finished")]
    CommandFinished {
        tab_id: u32,
        cmd: String,
        cwd: String,
        code: i32,
        /// 运行时长（毫秒），未知时为 0
        duration_ms: u64,
    },
}

/// 主题载荷（给前端用的扁平结构，颜色都是 CSS 字符串）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemePayload {
    pub name: String,
    pub bg: String,
    pub fg: String,
    pub accent: String,
    pub muted: String,
    pub success: String,
    pub error: String,
    pub block_border: String,
    pub title_bg: String,
    pub sidebar_bg: String,
    pub tab_bg: String,
    pub is_dark: bool,
}

impl ServerMessage {
    /// 序列化成 JSON 字符串
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }
}

/// 编码字节为 base64
pub fn encode_b64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// 解码 base64 为字节
pub fn decode_b64(s: &str) -> Result<Vec<u8>, base64::DecodeError> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(s)
}

#[cfg(test)]
mod tests {
    use super::{ClientMessage, NotificationKind};

    #[test]
    fn parses_frameless_window_controls() {
        let minimize: ClientMessage =
            serde_json::from_str(r#"{"type":"window_minimize"}"#).unwrap();
        assert!(matches!(minimize, ClientMessage::WindowMinimize));

        let maximize: ClientMessage =
            serde_json::from_str(r#"{"type":"window_toggle_maximize"}"#).unwrap();
        assert!(matches!(maximize, ClientMessage::WindowToggleMaximize));
    }

    #[test]
    fn parses_terminal_output_acknowledgement() {
        let parsed = serde_json::from_str::<ClientMessage>(
            r#"{"type":"output_ack","tab_id":7,"seq":3}"#,
        )
        .unwrap();

        assert!(matches!(
            parsed,
            ClientMessage::OutputAck { tab_id: 7, seq: 3 }
        ));
    }

    #[test]
    fn parses_desktop_notification_kind_and_defaults_missing_kind_to_command() {
        let agent = serde_json::from_str::<ClientMessage>(
            r#"{"type":"desktop_notify","kind":"agent","tab_id":7,"title":"完成","body":"codex"}"#,
        )
        .unwrap();
        assert!(matches!(
            agent,
            ClientMessage::DesktopNotify {
                kind: NotificationKind::Agent,
                tab_id: Some(7),
                ..
            }
        ));

        let legacy = serde_json::from_str::<ClientMessage>(
            r#"{"type":"desktop_notify","title":"完成","body":"make"}"#,
        )
        .unwrap();
        assert!(matches!(
            legacy,
            ClientMessage::DesktopNotify {
                kind: NotificationKind::Command,
                ..
            }
        ));
    }

    #[test]
    fn legacy_save_config_notification_setting_maps_to_agent_preference() {
        let parsed = serde_json::from_str::<ClientMessage>(
            r#"{"type":"save_config","config":{"theme":"lotus","shell":"","font":"JetBrains Mono","font_size":14,"opacity":1.0,"notifications_enabled":false}}"#,
        )
        .unwrap();

        let ClientMessage::SaveConfig { config } = parsed else {
            panic!("expected save_config message");
        };
        assert!(!config.agent_notifications_enabled);
        assert!(!config.command_notifications_enabled);
    }
}
