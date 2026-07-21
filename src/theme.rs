//! 主题系统 —— 定义 Lotus 的配色方案
//!
//! 颜色用 RGB 元组 (u8, u8, u8) 表示，前端通过 CSS 变量应用。
//! 这样 Rust 侧不依赖任何 UI 框架的颜色类型，前端拿到的就是纯数据。

/// RGB 颜色（不依赖任何 UI 框架）
pub type Rgb = (u8, u8, u8);

/// 转成 CSS 的 `rgb(r, g, b)` 字符串
pub fn rgb_to_css(c: Rgb) -> String {
    format!("rgb({}, {}, {})", c.0, c.1, c.2)
}

/// 转成十六进制 `#rrggbb`
#[allow(dead_code)]
pub fn rgb_to_hex(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c.0, c.1, c.2)
}

/// 一套完整的终端主题
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct Theme {
    /// 主题名（展示用）
    pub name: &'static str,
    /// 背景色（终端主区域）
    pub bg: Rgb,
    /// 前景色（默认文字）
    pub fg: Rgb,
    /// 强调色（命令提示符、边框、光标）
    pub accent: Rgb,
    /// 次要文字（状态栏、提示）
    pub muted: Rgb,
    /// 成功（命令执行成功）
    pub success: Rgb,
    /// 错误（命令失败）
    pub error: Rgb,
    /// 命令块边框
    pub block_border: Rgb,
    /// 标题栏背景
    pub title_bg: Rgb,
    /// 侧边栏背景
    pub sidebar_bg: Rgb,
    /// 标签栏背景
    pub tab_bg: Rgb,
    /// 是否是深色主题（影响前端某些样式调整）
    pub is_dark: bool,
}

impl Theme {
    /// Lotus 粉 —— 默认主题，柔和的暖色调，致敬莲花
    pub fn lotus() -> Self {
        Self {
            name: "lotus",
            bg: (30, 27, 38),            // 深紫黑
            fg: (230, 225, 235),         // 暖白
            accent: (232, 141, 167),     // 莲花粉
            muted: (130, 122, 145),      // 灰紫
            success: (152, 195, 121),    // 柔绿
            error: (224, 108, 117),      // 柔红
            block_border: (80, 70, 95),  // 暗紫边框
            title_bg: (38, 34, 50),      // 标题栏稍亮
            sidebar_bg: (25, 23, 33),    // 侧边栏稍暗
            tab_bg: (35, 31, 45),
            is_dark: true,
        }
    }

    /// Dracula —— 经典深紫色编程主题，广受欢迎
    pub fn dracula() -> Self {
        Self {
            name: "dracula",
            bg: (40, 42, 54),            // Dracula 经典深紫
            fg: (248, 248, 242),         // 接近白
            accent: (189, 147, 249),     // 紫色
            muted: (98, 114, 164),       // 注释色蓝灰
            success: (80, 250, 123),     // 亮绿
            error: (255, 85, 85),        // 亮红
            block_border: (68, 71, 90),  // 当前行色
            title_bg: (33, 34, 44),
            sidebar_bg: (28, 29, 38),
            tab_bg: (50, 52, 65),
            is_dark: true,
        }
    }

    /// 极简白 —— 干净明亮的浅色主题，白天使用
    pub fn light() -> Self {
        Self {
            name: "light",
            bg: (250, 249, 246),         // 暖白背景
            fg: (60, 56, 70),            // 深灰文字
            accent: (200, 85, 120),      // 玫红强调（lotus 粉的深色版）
            muted: (140, 135, 150),      // 中灰
            success: (100, 140, 80),     // 深绿
            error: (200, 70, 80),        // 深红
            block_border: (220, 218, 215),
            title_bg: (243, 241, 237),
            sidebar_bg: (246, 244, 240),
            tab_bg: (248, 247, 244),
            is_dark: false,
        }
    }

    /// 从内置主题名获取
    pub fn by_name(name: &str) -> Self {
        match name {
            "lotus" => Self::lotus(),
            "dracula" => Self::dracula(),
            "light" => Self::light(),
            _ => Self::lotus(),
        }
    }

    /// 列出所有内置主题名（给前端下拉框用）
    pub fn list() -> Vec<&'static str> {
        vec!["lotus", "dracula", "light"]
    }
}

impl Default for Theme {
    fn default() -> Self {
        Self::lotus()
    }
}
