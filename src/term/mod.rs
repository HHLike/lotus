//! 终端模块：管理多个 tab 的 PTY 生命周期

pub mod manager;

pub use manager::{TermEvent, TermManager};
