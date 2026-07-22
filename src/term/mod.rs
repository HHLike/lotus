//! 终端模块：管理多个 tab 的 PTY 生命周期

pub mod manager;
pub mod output_flow;

pub use manager::{TermEvent, TermManager};
pub use output_flow::OutputFlow;
