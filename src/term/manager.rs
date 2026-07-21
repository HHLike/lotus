//! 终端 Manager —— 管理多个 tab 的 PTY 生命周期
//!
//! 每个 tab 对应一个独立的 bash 子进程（通过 portable-pty spawn）。
//! 所有 tab 的 PTY 输出通过一个统一的 channel 汇总到主循环，
//! 主循环再批量下发给前端。

use anyhow::{Context, Result};
use log::error;
use std::collections::HashMap;
use std::sync::mpsc;

use crate::pty::{spawn_shell, PtyHandle, PtyOutput};
use crate::shell_integration::{parse_osc9_payload, CommandEvent, CommandInfo};

/// 单个 tab 的事件（从 Manager 发给主循环）
#[derive(Debug)]
pub enum TermEvent {
    /// PTY 输出（已转 base64）
    Output { tab_id: u32, data: Vec<u8> },
    /// 子进程退出
    Exited { tab_id: u32 },
    /// 命令开始（preexec / OSC 9 start）
    CommandStart {
        tab_id: u32,
        cmd: String,
        cwd: String,
    },
    /// 命令执行完（从 OSC 9 序列解析）
    CommandRun {
        tab_id: u32,
        cmd: String,
        cwd: String,
        code: i32,
    },
}

/// 一个 tab 的运行时状态
struct Tab {
    /// PTY 句柄（用于写输入、resize、kill）
    handle: PtyHandle,
    /// 当前标题（从 OSC 序列解析，或默认用 cwd）
    #[allow(dead_code)]
    title: String,
    /// 该 tab 所属的项目 id（用于切换项目时过滤显示，不杀 PTY）
    project_id: u32,
}

/// 终端 Manager：管理所有 tab
pub struct TermManager {
    /// 下一个 tab id
    next_id: u32,
    /// 所有活跃的 tab
    tabs: HashMap<u32, Tab>,
    /// shell 路径
    shell: String,
    /// 默认 cwd
    default_cwd: String,
    /// shell 集成脚本路径（None 表示不启用）
    init_file: Option<std::path::PathBuf>,
    /// 给 reader 线程用的 sender（每个 tab spawn 时 clone 一份）
    event_tx: mpsc::Sender<TermEvent>,
}

impl TermManager {
    /// 创建 Manager。`event_tx` 是 TermEvent 的接收端，由主循环持有。
    pub fn new(
        shell: String,
        default_cwd: String,
        event_tx: mpsc::Sender<TermEvent>,
        init_file: Option<std::path::PathBuf>,
    ) -> Self {
        Self {
            next_id: 1,
            tabs: HashMap::new(),
            shell,
            default_cwd,
            init_file,
            event_tx,
        }
    }

    /// 新建一个 tab，返回 tab_id
    /// 新建一个 tab，返回 tab_id
    /// `cwd_override`: None 用 default_cwd，Some 用指定目录
    /// `project_id`: 该 tab 所属的项目（切换项目时按此过滤）
    pub fn create_tab(
        &mut self,
        cols: u16,
        rows: u16,
        cwd_override: Option<&str>,
        project_id: u32,
    ) -> Result<u32> {
        let tab_id = self.next_id;
        self.next_id += 1;
        let cwd = cwd_override.unwrap_or(&self.default_cwd);

        // 每个 tab 用自己的 PtyOutput channel 桥接到统一的 TermEvent channel
        let (pty_tx, pty_rx) = mpsc::channel::<PtyOutput>();
        let handle = spawn_shell(
            &self.shell,
            cols,
            rows,
            pty_tx,
            self.init_file.as_deref(),
            cwd,
        )
        .with_context(|| format!("启动 shell `{}` 失败", self.shell))?;

        // 桥接线程：把 PtyOutput 转成 TermEvent::Output（同步，无需 tokio）
        // 同时扫描 OSC 9 序列，解析出命令信息
        let event_tx = self.event_tx.clone();
        std::thread::Builder::new()
            .name(format!("lotus-bridge-{}", tab_id))
            .spawn(move || {
                // OSC 9 解析器状态机（处理跨 chunk 的序列）
                let mut osc_parser = Osc9Parser::new();
                // pty_rx.recv() 返回 None 表示 PTY reader 线程结束（子进程退出）
                while let Ok(PtyOutput(bytes)) = pty_rx.recv() {
                    // 1. 先转发原始字节给终端渲染（OSC 序列 xterm.js 会忽略）
                    if event_tx
                        .send(TermEvent::Output {
                            tab_id,
                            data: bytes.clone(),
                        })
                        .is_err()
                    {
                        break;
                    }
                    // 2. 扫描同一批字节，提取 OSC 9 命令信息
                    for info in osc_parser.feed(&bytes) {
                        let ev = match info.event {
                            CommandEvent::Start => TermEvent::CommandStart {
                                tab_id,
                                cmd: info.cmd,
                                cwd: info.cwd,
                            },
                            CommandEvent::End => TermEvent::CommandRun {
                                tab_id,
                                cmd: info.cmd,
                                cwd: info.cwd,
                                code: info.code,
                            },
                        };
                        let _ = event_tx.send(ev);
                    }
                }
                // PTY reader 结束 → 子进程退出
                let _ = event_tx.send(TermEvent::Exited { tab_id });
            })
            .ok();

        let title = self.default_cwd.clone();
        self.tabs.insert(
            tab_id,
            Tab {
                handle,
                title,
                project_id,
            },
        );

        Ok(tab_id)
    }

    /// 写输入到指定 tab
    pub fn write(&mut self, tab_id: u32, bytes: &[u8]) -> Result<()> {
        match self.tabs.get_mut(&tab_id) {
            Some(tab) => tab.handle.write(bytes),
            None => {
                error!("write: tab {} 不存在", tab_id);
                Ok(())
            }
        }
    }

    /// 调整指定 tab 的 PTY 大小
    pub fn resize(&mut self, tab_id: u32, cols: u16, rows: u16) -> Result<()> {
        match self.tabs.get(&tab_id) {
            Some(tab) => tab.handle.resize(cols, rows),
            None => Ok(()),
        }
    }

    /// 关闭指定 tab（杀子进程）
    pub fn close_tab(&mut self, tab_id: u32) {
        if let Some(mut tab) = self.tabs.remove(&tab_id) {
            let _ = tab.handle.kill();
        }
    }

    /// 更新 tab 标题（OSC 序列解析后调用）
    #[allow(dead_code)]
    pub fn set_title(&mut self, tab_id: u32, title: String) {
        if let Some(tab) = self.tabs.get_mut(&tab_id) {
            tab.title = title;
        }
    }

    /// 设置默认 cwd（切换项目时调用）
    pub fn set_default_cwd(&mut self, cwd: String) {
        self.default_cwd = cwd;
    }

    /// 列出所有 tab id（切换项目时关闭所有 tab 用）
    pub fn list_tab_ids(&self) -> Vec<u32> {
        self.tabs.keys().copied().collect()
    }

    /// 判断某项目是否还有存活的 tab（切换项目时空项目判断用）
    pub fn has_tabs_for_project(&self, project_id: u32) -> bool {
        self.tabs.values().any(|t| t.project_id == project_id)
    }

    /// 关闭所有 tab（退出/切换项目时调用）
    pub fn close_all(&mut self) {
        for (_, mut tab) in self.tabs.drain() {
            let _ = tab.handle.kill();
        }
    }
}

/// OSC 9 序列解析器（状态机，处理跨 chunk 的序列）
///
/// OSC 9 格式：`\e]9;<payload>\e\\` 或 `\e]9;<payload>\x07`（BEL 终止）
/// 其中 <payload> 是 JSON：`{"cmd":"...","cwd":"...","code":0}`
///
/// 重要：解析出的 OSC 序列仍然会作为原始字节转发给 xterm.js（xterm.js 自己会
/// 忽略未识别的 OSC 9），这里只是"旁路监听"提取命令信息。
struct Osc9Parser {
    /// 状态机当前状态
    state: OscState,
    /// 当前正在累积的 payload
    payload: Vec<u8>,
}

#[derive(PartialEq)]
enum OscState {
    /// 正常文本（扫描 ESC）
    Normal,
    /// 看到 ESC，等 ]
    Esc,
    /// 看到 ESC ]，等 '9'
    OscStart1,
    /// 看到 ESC ] 9，等 ';'
    OscStart2,
    /// 在 payload 中，等 ST（ESC \）或 BEL（0x07）
    InPayload,
    /// 看到 payload 中的 ESC，等 '\'（ST 的第二字节）
    PayloadEsc,
}

impl Osc9Parser {
    fn new() -> Self {
        Self {
            state: OscState::Normal,
            payload: Vec::new(),
        }
    }

    /// 喂入一批字节，返回所有完整解析出的 CommandInfo
    fn feed(&mut self, bytes: &[u8]) -> Vec<CommandInfo> {
        let mut results = Vec::new();
        for &b in bytes {
            self.state = match (&self.state, b) {
                // Normal: 找 ESC (0x1b)
                (OscState::Normal, 0x1b) => OscState::Esc,
                (OscState::Normal, _) => OscState::Normal,

                // Esc: 找 ']'
                (OscState::Esc, b']') => OscState::OscStart1,
                (OscState::Esc, _) => OscState::Normal,

                // OscStart1: 找 '9'
                (OscState::OscStart1, b'9') => OscState::OscStart2,
                (OscState::OscStart1, 0x1b) => OscState::Esc,
                (OscState::OscStart1, _) => OscState::Normal,

                // OscStart2: 找 ';'
                (OscState::OscStart2, b';') => {
                    self.payload.clear();
                    OscState::InPayload
                }
                (OscState::OscStart2, 0x1b) => OscState::Esc,
                (OscState::OscStart2, _) => OscState::Normal,

                // InPayload: 累积字节，等 ST（ESC \）或 BEL（0x07）
                (OscState::InPayload, 0x07) => {
                    // BEL 终止 → 解析 payload
                    if let Some(info) = parse_osc9_payload(&self.payload) {
                        results.push(info);
                    }
                    self.payload.clear();
                    OscState::Normal
                }
                (OscState::InPayload, 0x1b) => OscState::PayloadEsc,
                (OscState::InPayload, b) => {
                    self.payload.push(b);
                    OscState::InPayload
                }

                // PayloadEsc: payload 中看到 ESC，等 '\'（ST 第二字节）
                (OscState::PayloadEsc, b'\\') => {
                    // ST 终止 → 解析 payload
                    if let Some(info) = parse_osc9_payload(&self.payload) {
                        results.push(info);
                    }
                    self.payload.clear();
                    OscState::Normal
                }
                (OscState::PayloadEsc, 0x1b) => {
                    // 连续 ESC，重新进入 PayloadEsc
                    OscState::PayloadEsc
                }
                (OscState::PayloadEsc, b) => {
                    // 不是 ST，把 ESC 和当前字节都计入 payload
                    self.payload.push(0x1b);
                    self.payload.push(b);
                    OscState::InPayload
                }
            };
        }
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_osc9_complete_sequence() {
        let mut parser = Osc9Parser::new();
        // 一条完整的 OSC 9 序列（ST 终止）
        let seq = b"\x1b]9;{\"cmd\":\"ls\",\"cwd\":\"/home\",\"code\":0}\x1b\\";
        let results = parser.feed(seq);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].cmd, "ls");
        assert_eq!(results[0].cwd, "/home");
        assert_eq!(results[0].code, 0);
    }

    #[test]
    fn parse_osc9_bel_terminated() {
        let mut parser = Osc9Parser::new();
        let seq = b"\x1b]9;{\"cmd\":\"echo hi\",\"cwd\":\"/tmp\",\"code\":0}\x07";
        let results = parser.feed(seq);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].cmd, "echo hi");
    }

    #[test]
    fn parse_osc9_split_across_chunks() {
        let mut parser = Osc9Parser::new();
        // 序列被拆成两个 chunk
        let part1 = b"\x1b]9;{\"cmd\":\"ls\"";
        let part2 = b",\"cwd\":\"/home\",\"code\":0}\x1b\\";
        assert_eq!(parser.feed(part1).len(), 0);
        let results = parser.feed(part2);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].cmd, "ls");
    }

    #[test]
    fn parse_ignores_non_osc_bytes() {
        let mut parser = Osc9Parser::new();
        // 混在普通文本里
        let bytes = b"hello world\x1b]9;{\"cmd\":\"ls\",\"cwd\":\"/\",\"code\":0}\x1b\\more text";
        let results = parser.feed(bytes);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].cmd, "ls");
    }

    #[test]
    fn parse_multiple_in_one_chunk() {
        let mut parser = Osc9Parser::new();
        let bytes = b"\x1b]9;{\"cmd\":\"a\",\"cwd\":\"/\",\"code\":0}\x1b\\\x1b]9;{\"cmd\":\"b\",\"cwd\":\"/\",\"code\":1}\x1b\\";
        let results = parser.feed(bytes);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].cmd, "a");
        assert_eq!(results[1].cmd, "b");
        assert_eq!(results[1].code, 1);
    }
}

