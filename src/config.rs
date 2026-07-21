//! 配置加载与持久化 —— 从 ~/.config/lotus/config.toml 读写用户配置
//!
//! 所有新字段都带 serde default，向后兼容旧配置文件。

use anyhow::{Context, Result};
use log::{info, warn};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 顶层配置（可序列化，给设置面板读写用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// 主题名（参考 theme::Theme::by_name）
    #[serde(default = "default_theme")]
    pub theme: String,
    /// 启动时调用的 shell，None 表示用 $SHELL 或回退到 bash
    #[serde(default)]
    pub shell: Option<String>,
    /// 终端字体（必须是系统已装的等宽字体名）
    #[serde(default = "default_font")]
    pub font: String,
    /// 字号
    #[serde(default = "default_font_size")]
    pub font_size: u16,
    /// 窗口透明度 0.5~1.0（1.0 = 完全不透明）
    #[serde(default = "default_opacity")]
    pub opacity: f32,
    /// 上次打开的项目 id（启动时自动恢复，None 表示用第一个项目）
    #[serde(default)]
    pub last_project_id: Option<u32>,
}

fn default_theme() -> String {
    "lotus".to_string()
}
fn default_font() -> String {
    "JetBrains Mono".to_string()
}
fn default_font_size() -> u16 {
    14
}
fn default_opacity() -> f32 {
    1.0
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            shell: None,
            font: default_font(),
            font_size: default_font_size(),
            opacity: default_opacity(),
            last_project_id: None,
        }
    }
}

impl Config {
    /// 返回配置文件的标准路径：~/.config/lotus/config.toml
    pub fn config_path() -> Option<PathBuf> {
        Some(Self::config_dir()?.join("config.toml"))
    }

    /// 返回配置目录：~/.config/lotus
    pub fn config_dir() -> Option<PathBuf> {
        let home = std::env::var("HOME").ok()?;
        Some(PathBuf::from(home).join(".config").join("lotus"))
    }

    /// 从默认路径加载；文件不存在或解析失败时返回默认配置
    pub fn load() -> Self {
        match Self::config_path() {
            Some(path) if path.exists() => Self::load_from(&path),
            _ => Config::default(),
        }
    }

    /// 从指定路径加载
    fn load_from(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => match toml::from_str::<Self>(&text) {
                Ok(cfg) => cfg,
                Err(e) => {
                    warn!("配置文件 {} 解析失败：{}，使用默认配置", path.display(), e);
                    Config::default()
                }
            },
            Err(e) => {
                warn!("读取配置文件 {} 失败：{}，使用默认配置", path.display(), e);
                Config::default()
            }
        }
    }

    /// 保存到 ~/.config/lotus/config.toml
    pub fn save(&self) -> Result<()> {
        let dir = Self::config_dir().context("无法确定配置目录（$HOME 未设置）")?;
        std::fs::create_dir_all(&dir).with_context(|| format!("创建目录 {} 失败", dir.display()))?;

        let path = Self::config_path().context("无法确定配置文件路径")?;
        let text = toml::to_string_pretty(self).context("序列化配置失败")?;
        std::fs::write(&path, text).with_context(|| format!("写入 {} 失败", path.display()))?;
        info!("配置已保存到 {}", path.display());
        Ok(())
    }

    /// 解析实际要启动的 shell 命令
    /// 优先级：配置文件 > $SHELL 环境变量 > bash
    pub fn resolve_shell(&self) -> String {
        if let Some(s) = &self.shell {
            if !s.is_empty() {
                return s.clone();
            }
        }
        if let Ok(s) = std::env::var("SHELL") {
            if !s.is_empty() {
                return s;
            }
        }
        "bash".to_string()
    }
}
