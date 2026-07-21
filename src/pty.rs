//! PTY 处理 —— 启动子 shell 并在独立线程里读取其输出
//!
//! 关键设计：
//! - `portable-pty` 的读取是**阻塞**的，绝不能放在渲染线程
//! - 这里启动一个专属 OS 线程读 PTY，通过 `tokio::sync::mpsc` 把字节流
//!   转发到主异步循环；写入则直接通过共享的 writer 句柄同步发送
//! - `PtyHandle` 持有 writer 和子进程句柄，App 通过它向 shell 发送输入/调整大小

use anyhow::{Context, Result};
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};
use std::io::Write;
use std::sync::mpsc;
use std::sync::Arc;

/// PTY 输出事件：从子进程读到的原始字节（可能含 ANSI 转义）
pub struct PtyOutput(pub Vec<u8>);

/// PTY 句柄：App 通过它与子进程交互
pub struct PtyHandle {
    /// 主 PTY（用于 resize）
    pty: Arc<dyn MasterPty + Send>,
    /// writer 句柄（长期持有，避免反复 take/drop 导致向 slave 发 EOF）
    /// 注意：portable-pty 文档明确——drop writer 会向 slave 发送 EOF，
    ///       且 take_writer 只能调用一次。所以必须缓存。
    writer: Box<dyn Write + Send>,
    /// 子进程
    child: Box<dyn portable_pty::Child>,
}

impl PtyHandle {
    /// 向 shell 写入原始字节（键盘输入、命令等）
    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.writer.write_all(bytes)?;
        self.writer.flush()?;
        Ok(())
    }

    /// 调整子进程的终端窗口大小
    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        self.pty
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("PTY resize 失败")?;
        Ok(())
    }

    /// 请求子进程退出（温和关闭）
    #[allow(dead_code)]
    pub fn kill(&mut self) -> Result<()> {
        let _ = self.child.kill();
        Ok(())
    }
}

/// 启动一个子 shell。
///
/// - `shell_cmd`: 要执行的 shell（如 `/bin/bash`）
/// - `cols`/`rows`: 初始窗口大小
/// - `tx`: PTY 输出转发通道
/// - `init_file`: bash 集成脚本路径（None 表示不启用 shell integration）
/// - `cwd`: shell 的工作目录（项目目录）
///
/// 返回 `PtyHandle` 给调用方持有（用于写输入/resize）。
pub fn spawn_shell(
    shell_cmd: &str,
    cols: u16,
    rows: u16,
    tx: mpsc::Sender<PtyOutput>,
    init_file: Option<&std::path::Path>,
    cwd: &str,
) -> Result<PtyHandle> {
    let pty_system = native_pty_system();

    // 打开 PTY（指定初始大小）
    let pair = pty_system
        .openpty(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("打开 PTY 失败")?;

    // 构造子进程命令
    let mut cmd = CommandBuilder::new(shell_cmd);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLORTERM", "truecolor");
    // 用传入的 cwd（项目目录），目录不存在时回退到 lotus 进程目录
    let cwd_path = std::path::PathBuf::from(cwd);
    if cwd_path.exists() {
        cmd.cwd(cwd_path);
    } else {
        log::warn!("项目目录 {} 不存在，回退到当前目录", cwd);
        cmd.cwd(std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("/")));
    }

    // 如果是 bash 且提供了 init-file，加 --init-file 参数启用 shell integration
    // （只对 bash 生效；zsh/fish 跳过，命令历史功能降级）
    if let Some(path) = init_file {
        let is_bash = shell_cmd.contains("bash");
        if is_bash && path.exists() {
            cmd.arg("--init-file");
            cmd.arg(path);
            log::info!("启用 shell integration: --init-file {}", path.display());
        }
    }

    let child = pair
        .slave
        .spawn_command(cmd)
        .with_context(|| format!("启动 shell `{}` 失败", shell_cmd))?;

    // 释放 slave，让子进程能检测到我们关闭时 EOF
    drop(pair.slave);

    // 取 reader（输出流），放到独立线程里持续读取
    let mut reader = pair
        .master
        .try_clone_reader()
        .context("克隆 PTY reader 失败")?;

    // ⚠️ 关键：take_writer 只能调用一次，且 writer drop 会向 slave 发 EOF。
    // 必须在 master 转成 Arc 之前就取出，并长期持有在 PtyHandle 里。
    let writer = pair
        .master
        .take_writer()
        .context("获取 PTY writer 失败")?;

    // portable_pty 的 master 是 Box<dyn MasterPty + Send>，转成 Arc 便于共享
    let pty: Arc<dyn MasterPty + Send> = Arc::from(pair.master);

    std::thread::Builder::new()
        .name("lotus-pty-reader".into())
        .spawn(move || {
            // 4KB 缓冲，循环读；读到 EOF（子进程退出）就结束
            let mut buf = [0u8; 4096];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => {
                        log::info!("PTY reader: 收到 EOF（子进程退出）");
                        break;
                    }
                    Ok(n) => {
                        if tx.send(PtyOutput(buf[..n].to_vec())).is_err() {
                            log::info!("PTY reader: 接收方已关闭，退出");
                            break;
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => {
                        log::error!("PTY reader: 读错误 {}，退出", e);
                        break;
                    }
                }
            }
        })
        .context("启动 PTY reader 线程失败")?;

    Ok(PtyHandle { pty, writer, child })
}
