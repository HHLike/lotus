//! 数据持久化 —— 命令历史 + 目录书签
//!
//! 存储位置（XDG）：~/.local/share/lotus/
//! - history.json   命令历史（最多 MAX_HISTORY 条）
//! - bookmarks.json 目录书签
//!
//! 写入用原子写（temp + rename），避免崩溃损坏。

use anyhow::{Context, Result};
use log::warn;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::shell_integration::data_dir;

const MAX_HISTORY: usize = 1000;
const MAX_RECENTS: usize = 5;

// ============================ 历史记录 ============================

/// 单条历史记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    /// 命令文本
    pub cmd: String,
    /// 执行时的工作目录
    pub cwd: String,
    /// Unix 时间戳（秒）
    pub ts: u64,
    /// 退出码
    pub code: i32,
}

/// 历史存储（内存 + 持久化）
#[derive(Debug, Clone, Default)]
pub struct HistoryStore {
    pub entries: Vec<HistoryEntry>,
}

impl HistoryStore {
    /// 从磁盘加载；文件不存在/损坏时返回空
    #[allow(dead_code)]
    pub fn load() -> Self {
        match Self::path() {
            Some(p) if p.exists() => match std::fs::read_to_string(&p) {
                Ok(text) => match serde_json::from_str::<Vec<HistoryEntry>>(&text) {
                    Ok(entries) => Self { entries },
                    Err(e) => {
                        warn!("历史文件 {} 解析失败：{}", p.display(), e);
                        Self::default()
                    }
                },
                Err(_) => Self::default(),
            },
            _ => Self::default(),
        }
    }

    /// 追加一条历史，超过上限自动裁剪
    pub fn append(&mut self, entry: HistoryEntry) {
        self.entries.push(entry);
        // 超过上限：保留最近的 MAX_HISTORY 条
        if self.entries.len() > MAX_HISTORY {
            let drop_count = self.entries.len() - MAX_HISTORY;
            self.entries.drain(0..drop_count);
        }
    }

    /// 搜索（简单子串匹配，返回最多 100 条，按时间倒序）
    pub fn search(&self, query: &str) -> Vec<&HistoryEntry> {
        let q = query.trim();
        let iter: Box<dyn Iterator<Item = &HistoryEntry>> = if q.is_empty() {
            Box::new(self.entries.iter())
        } else {
            Box::new(self.entries.iter().filter(|e| e.cmd.contains(q)))
        };
        // 倒序（最新在前），最多 100 条
        let mut result: Vec<&HistoryEntry> = iter.collect();
        result.reverse();
        result.truncate(100);
        result
    }

    /// 清空
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// 保存到磁盘（原子写）
    pub fn save(&self) -> Result<()> {
        let path = Self::path().context("无法确定历史文件路径")?;
        atomic_write_json(&path, &self.entries)
    }

    /// 保存到指定项目目录（per-project）
    pub fn save_in(&self, project_dir: &PathBuf) -> Result<()> {
        std::fs::create_dir_all(project_dir)
            .with_context(|| format!("创建项目目录失败: {}", project_dir.display()))?;
        atomic_write_json(&project_dir.join("history.json"), &self.entries)
    }

    fn path() -> Option<PathBuf> {
        Some(data_dir()?.join("history.json"))
    }

    /// 从指定项目目录加载
    pub fn load_in(project_dir: &PathBuf) -> Self {
        let path = project_dir.join("history.json");
        if !path.exists() {
            return Self::default();
        }
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str::<Vec<HistoryEntry>>(&text).map(|e| Self { entries: e }).unwrap_or_else(|e| {
                warn!("历史文件 {} 解析失败：{}", path.display(), e);
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    /// 计算最近访问的目录（从历史 cwd 字段派生，去重，取最近 MAX_RECENTS 个）
    pub fn recent_dirs(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        let mut result = Vec::new();
        // 从最新往回扫
        for e in self.entries.iter().rev() {
            if seen.insert(e.cwd.clone()) {
                result.push(e.cwd.clone());
                if result.len() >= MAX_RECENTS {
                    break;
                }
            }
        }
        result
    }
}

// ============================ 书签 ============================

/// 单个书签
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BookmarkEntry {
    pub id: u32,
    pub name: String,
    pub path: String,
}

/// 书签存储
#[derive(Debug, Clone)]
pub struct BookmarkStore {
    pub entries: Vec<BookmarkEntry>,
    next_id: u32,
}

impl BookmarkStore {
    pub fn load() -> Self {
        match Self::path() {
            Some(p) if p.exists() => match std::fs::read_to_string(&p) {
                Ok(text) => match serde_json::from_str::<Vec<BookmarkEntry>>(&text) {
                    Ok(entries) => {
                        let max_id = entries.iter().map(|e| e.id).max().unwrap_or(0);
                        Self {
                            entries,
                            next_id: max_id + 1,
                        }
                    }
                    Err(e) => {
                        warn!("书签文件 {} 解析失败：{}", p.display(), e);
                        Self::default()
                    }
                },
                Err(_) => Self::default(),
            },
            _ => Self::default(),
        }
    }

    /// 添加书签，返回新 id
    pub fn add(&mut self, name: String, path: String) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.entries.push(BookmarkEntry { id, name, path });
        id
    }

    /// 删除书签，返回是否删除成功
    pub fn remove(&mut self, id: u32) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| e.id != id);
        self.entries.len() < before
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path().context("无法确定书签文件路径")?;
        atomic_write_json(&path, &self.entries)
    }

    /// 保存到指定项目目录（per-project）
    pub fn save_in(&self, project_dir: &PathBuf) -> Result<()> {
        std::fs::create_dir_all(project_dir)
            .with_context(|| format!("创建项目目录失败: {}", project_dir.display()))?;
        atomic_write_json(&project_dir.join("bookmarks.json"), &self.entries)
    }

    fn path() -> Option<PathBuf> {
        Some(data_dir()?.join("bookmarks.json"))
    }

    /// 从指定项目目录加载
    pub fn load_in(project_dir: &PathBuf) -> Self {
        let path = project_dir.join("bookmarks.json");
        if !path.exists() {
            return Self::default();
        }
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str::<Vec<BookmarkEntry>>(&text).map(|e| {
                let max_id = e.iter().map(|b| b.id).max().unwrap_or(0);
                Self { entries: e, next_id: max_id + 1 }
            }).unwrap_or_else(|e| {
                warn!("书签文件 {} 解析失败：{}", path.display(), e);
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }
}

impl Default for BookmarkStore {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            next_id: 1,
        }
    }
}

// ============================ 工具：原子写 JSON ============================

/// 原子写：写到 .tmp 再 rename，避免崩溃损坏
fn atomic_write_json<T: Serialize>(path: &PathBuf, value: &T) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("创建目录 {} 失败", parent.display()))?;
    }
    let text = serde_json::to_string_pretty(value).context("JSON 序列化失败")?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, &text)
        .with_context(|| format!("写入临时文件 {} 失败", tmp.display()))?;
    std::fs::rename(&tmp, path)
        .with_context(|| format!("重命名 {} → {} 失败", tmp.display(), path.display()))?;
    Ok(())
}

/// 当前 Unix 时间戳（秒）
pub fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ============================ 项目（Workspace）============================

/// 单个项目的元数据（持久化到 projects.json）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMeta {
    pub id: u32,
    pub name: String,
    pub cwd: String,
}

/// 完整的项目（含运行时加载的历史/书签）
pub struct Project {
    pub meta: ProjectMeta,
    pub history: HistoryStore,
    pub bookmarks: BookmarkStore,
}

impl Project {
    /// 该项目的数据目录：~/.local/share/lotus/projects/<id>/
    pub fn dir(&self) -> Option<PathBuf> {
        Some(data_dir()?.join("projects").join(self.meta.id.to_string()))
    }

    /// 持久化历史 + 书签
    pub fn save_data(&self) -> Result<()> {
        let dir = self.dir().context("无法确定项目目录")?;
        self.history.save_in(&dir)?;
        self.bookmarks.save_in(&dir)?;
        Ok(())
    }
}

/// 项目存储（管理所有项目的元数据 + 当前项目的历史/书签缓存）
pub struct ProjectStore {
    /// 所有项目元数据（按 id 索引）
    pub metas: HashMap<u32, ProjectMeta>,
    /// 下一个项目 id
    next_id: u32,
}

impl ProjectStore {
    /// 加载：读 projects.json，首次启动自动创建默认项目 + 迁移旧数据
    pub fn load() -> Self {
        let mut store = Self::load_raw();
        // 如果没有任何项目，创建默认项目并迁移旧的 history/bookmarks
        if store.metas.is_empty() {
            let default_cwd = std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| "/".to_string());
            let default_id = store.create("默认项目".to_string(), default_cwd);
            // 迁移旧的 history.json / bookmarks.json（如果存在）
            if let Some(dir) = data_dir() {
                let old_history = dir.join("history.json");
                let old_bookmarks = dir.join("bookmarks.json");
                if let Some(proj_dir) = store.project_dir(default_id) {
                    let _ = std::fs::create_dir_all(&proj_dir);
                    if old_history.exists() {
                        let _ = std::fs::rename(&old_history, proj_dir.join("history.json"));
                    }
                    if old_bookmarks.exists() {
                        let _ = std::fs::rename(&old_bookmarks, proj_dir.join("bookmarks.json"));
                    }
                }
            }
            let _ = store.save_metadata();
            log::info!("已创建默认项目（id={}）并迁移旧数据", default_id);
        }
        store
    }

    fn load_raw() -> Self {
        let path = match data_dir() {
            Some(d) => d.join("projects").join("projects.json"),
            None => return Self::default(),
        };
        if !path.exists() {
            return Self::default();
        }
        match std::fs::read_to_string(&path) {
            Ok(text) => match serde_json::from_str::<Vec<ProjectMeta>>(&text) {
                Ok(metas_vec) => {
                    let max_id = metas_vec.iter().map(|m| m.id).max().unwrap_or(0);
                    let mut metas = HashMap::new();
                    for m in metas_vec {
                        metas.insert(m.id, m);
                    }
                    Self {
                        metas,
                        next_id: max_id + 1,
                    }
                }
                Err(e) => {
                    warn!("projects.json 解析失败：{}", e);
                    Self::default()
                }
            },
            Err(_) => Self::default(),
        }
    }

    /// 创建项目，返回新 id
    pub fn create(&mut self, name: String, cwd: String) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.metas.insert(
            id,
            ProjectMeta { id, name, cwd },
        );
        id
    }

    pub fn get(&self, id: u32) -> Option<&ProjectMeta> {
        self.metas.get(&id)
    }

    pub fn list(&self) -> Vec<&ProjectMeta> {
        let mut v: Vec<&ProjectMeta> = self.metas.values().collect();
        v.sort_by_key(|m| m.id);
        v
    }

    /// 删除项目，返回是否成功
    pub fn delete(&mut self, id: u32) -> bool {
        if self.metas.len() <= 1 {
            return false; // 至少保留一个项目
        }
        if self.metas.remove(&id).is_some() {
            // 删除项目数据目录
            if let Some(dir) = self.project_dir(id) {
                let _ = std::fs::remove_dir_all(&dir);
            }
            let _ = self.save_metadata();
            true
        } else {
            false
        }
    }

    /// 重命名项目
    pub fn rename(&mut self, id: u32, name: String) -> bool {
        if let Some(m) = self.metas.get_mut(&id) {
            m.name = name;
            let _ = self.save_metadata();
            true
        } else {
            false
        }
    }

    /// 持久化项目元数据到 projects.json
    pub fn save_metadata(&self) -> Result<()> {
        let dir = data_dir().context("无法确定数据目录")?;
        let projects_dir = dir.join("projects");
        std::fs::create_dir_all(&projects_dir)?;
        // 收集 owned 的元数据（按 id 排序）
        let mut metas_vec: Vec<ProjectMeta> = self.metas.values().cloned().collect();
        metas_vec.sort_by_key(|m| m.id);
        atomic_write_json(&projects_dir.join("projects.json"), &metas_vec)
    }

    /// 项目数据目录：~/.local/share/lotus/projects/<id>/
    pub fn project_dir(&self, id: u32) -> Option<PathBuf> {
        Some(data_dir()?.join("projects").join(id.to_string()))
    }

    /// 加载指定项目的完整数据（历史 + 书签）
    pub fn load_project(&self, id: u32) -> Option<Project> {
        let meta = self.metas.get(&id)?.clone();
        let dir = self.project_dir(id)?;
        let history = HistoryStore::load_in(&dir);
        let bookmarks = BookmarkStore::load_in(&dir);
        Some(Project { meta, history, bookmarks })
    }
}

impl Default for ProjectStore {
    fn default() -> Self {
        Self {
            metas: HashMap::new(),
            next_id: 1,
        }
    }
}
