//! Shell 集成 —— 通过自定义 bash init-file + OSC 9 序列捕获命令
//!
//! 原理（VS Code/iTerm 同款 shell integration）：
//! 1. Lotus 启动 bash 时指定 --init-file 指向我们的脚本
//! 2. 脚本先 source /etc/bash.bashrc + ~/.bashrc（保留用户环境）
//! 3. 再注入 preexec（DEBUG trap）+ PROMPT_COMMAND hook
//! 4. PTY reader 解析 OSC 9 → 提取 cmd/cwd/exit_code / start 事件
//!
//! OSC 9 序列格式：
//! - 开始：`\e]9;{"event":"start","cmd":"ls","cwd":"/home"}\e\\`
//! - 结束：`\e]9;{"event":"end","cmd":"ls","cwd":"/home","code":0}\e\\`
//!   （兼容旧格式：无 event 字段时视为 end）

use anyhow::{Context, Result};
use std::path::PathBuf;

/// 返回内嵌的 bash 集成脚本内容
pub fn integration_script() -> &'static str {
    r#"# Lotus Shell Integration —— 自动生成，请勿手动编辑
# 1. 先 source 系统和用户的 bashrc，保留完整环境
[ -f /etc/bash.bashrc ] && . /etc/bash.bashrc
[ -f ~/.bashrc ] && . ~/.bashrc

# 避免在非交互场景重复注入
[ -n "${__lotus_integration_loaded:-}" ] && return
__lotus_integration_loaded=1

__lotus_json_str() {
  printf '%s' "$1" | python3 -c 'import json,sys;print(json.dumps(sys.stdin.read()))' 2>/dev/null || printf '""'
}

# 2. 命令开始（preexec）：bash DEBUG trap
__lotus_preexec() {
  # 跳过 prompt / completion / 空命令
  [ -n "${COMP_LINE:-}" ] && return
  [ -z "${BASH_COMMAND:-}" ] && return
  case "$BASH_COMMAND" in
    __lotus_prompt*|__lotus_preexec*|PROMPT_COMMAND*) return ;;
  esac
  # 同一个命令在 DEBUG 里可能触发多次，做简单去重
  if [ "${__lotus_last_start_cmd:-}" = "$BASH_COMMAND" ] && [ "${__lotus_in_cmd:-0}" = "1" ]; then
    return
  fi
  __lotus_in_cmd=1
  __lotus_last_start_cmd="$BASH_COMMAND"
  local cmd_json
  cmd_json=$(__lotus_json_str "$BASH_COMMAND")
  printf '\e]9;{"event":"start","cmd":%s,"cwd":"%s"}\e\\' "$cmd_json" "$PWD"
}

# 3. 命令结束（prompt 前）：上报 exit code
__lotus_prompt() {
  local ec=$?
  # 提取最后一条命令（去掉 history 的序号前缀）
  local cmd
  cmd=$(HISTTIMEFORMAT= history 1 | sed 's/^ *[0-9]* *//')
  local cmd_json
  cmd_json=$(__lotus_json_str "$cmd")
  printf '\e]9;{"event":"end","cmd":%s,"cwd":"%s","code":%d}\e\\' "$cmd_json" "$PWD" "$ec"
  __lotus_in_cmd=0
  __lotus_last_start_cmd=""
}

# 4. 安装 hook
trap '__lotus_preexec' DEBUG
PROMPT_COMMAND="__lotus_prompt; ${PROMPT_COMMAND:-}"
"#
}

/// Pi 常驻会话的单次 Agent 工作开始/完成信号。
pub fn pi_notification_extension() -> &'static str {
    r#"export default function (pi) {
  const active = () => process.stdout.write("\x1b]9;4;3\x07");
  const clear = () => process.stdout.write("\x1b]9;4;0;\x07");

  pi.on("agent_start", active);
  pi.on("agent_settled", clear);
}
"#
}

/// 把集成脚本写到 ~/.local/share/lotus/shell-integration.bash（幂等）
pub fn install() -> Result<PathBuf> {
    let dir = data_dir().context("无法确定数据目录（$HOME 未设置）")?;
    std::fs::create_dir_all(&dir).with_context(|| format!("创建 {} 失败", dir.display()))?;
    let path = dir.join("shell-integration.bash");
    // 幂等：每次启动都覆盖，确保脚本是最新的（用户不会手动改这个文件）
    std::fs::write(&path, integration_script())
        .with_context(|| format!("写入 {} 失败", path.display()))?;
    Ok(path)
}

/// 写入由 Lotus 启动 Pi 时显式加载的通知扩展。
pub fn install_pi_notification_extension() -> Result<PathBuf> {
    let dir = data_dir().context("无法确定数据目录（$HOME 未设置）")?;
    std::fs::create_dir_all(&dir).with_context(|| format!("创建 {} 失败", dir.display()))?;
    let path = dir.join("pi-notification-extension.js");
    std::fs::write(&path, pi_notification_extension())
        .with_context(|| format!("写入 {} 失败", path.display()))?;
    Ok(path)
}

/// 返回数据目录 ~/.local/share/lotus
pub fn data_dir() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    Some(PathBuf::from(home).join(".local").join("share").join("lotus"))
}

/// 从 PTY 字节流解析出的命令信息
#[derive(Debug, Clone)]
pub struct CommandInfo {
    /// start / end
    pub event: CommandEvent,
    pub cmd: String,
    pub cwd: String,
    pub code: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandEvent {
    Start,
    End,
    AgentStart,
    AgentEnd,
}

/// 解析单个 OSC 9 payload（JSON 部分），失败返回 None
pub fn parse_osc9_payload(payload: &[u8]) -> Option<CommandInfo> {
    let text = std::str::from_utf8(payload).ok()?;

    let progress_event = match text.trim_end_matches(';') {
        "4;3" => Some(CommandEvent::AgentStart),
        "4;0" => Some(CommandEvent::AgentEnd),
        _ => None,
    };
    if let Some(event) = progress_event {
        return Some(CommandInfo {
            event,
            cmd: String::new(),
            cwd: String::new(),
            code: 0,
        });
    }

    #[derive(serde::Deserialize)]
    struct Osc9Payload {
        #[serde(default)]
        event: Option<String>,
        cmd: String,
        cwd: String,
        #[serde(default)]
        code: i32,
    }

    let p: Osc9Payload = serde_json::from_str(text).ok()?;
    let event = match p.event.as_deref() {
        Some("start") => CommandEvent::Start,
        // 缺省 / end / 旧格式 → 视为命令结束
        _ => CommandEvent::End,
    };
    Some(CommandInfo {
        event,
        cmd: p.cmd,
        cwd: p.cwd,
        code: p.code,
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_osc9_payload, pi_notification_extension, CommandEvent};

    #[test]
    fn parses_standard_terminal_progress_as_agent_lifecycle() {
        let started = parse_osc9_payload(b"4;3").unwrap();
        let finished = parse_osc9_payload(b"4;0;").unwrap();

        assert_eq!(started.event, CommandEvent::AgentStart);
        assert_eq!(finished.event, CommandEvent::AgentEnd);
    }

    #[test]
    fn pi_extension_emits_progress_for_complete_agent_turns() {
        let script = pi_notification_extension();

        assert!(script.contains("agent_start"));
        assert!(script.contains("agent_settled"));
        assert!(!script.contains("session_shutdown"));
        assert!(script.contains("\\x1b]9;4;3\\x07"));
        assert!(script.contains("\\x1b]9;4;0;\\x07"));
    }
}
