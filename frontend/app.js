// Lotus 🪷 前端逻辑
// 管理多个 xterm.js 终端实例，通过 IPC 与 Rust 后端通信

// ====== 全局错误捕获（白屏时把错误显示到页面上，便于诊断）======
window.addEventListener('error', (e) => {
  showErrorOverlay(`JS 错误: ${e.message}\n  at ${e.filename}:${e.lineno}:${e.colno}`);
});
window.addEventListener('unhandledrejection', (e) => {
  showErrorOverlay(`Promise 未捕获: ${e.reason}`);
});
function showErrorOverlay(msg) {
  const existing = document.getElementById('error-overlay');
  if (existing) existing.remove();
  const div = document.createElement('div');
  div.id = 'error-overlay';
  div.style.cssText = `
    position: fixed; top: 10px; left: 10px; right: 10px;
    background: #e06c75; color: white; padding: 12px 16px;
    border-radius: 8px; font-family: monospace; font-size: 12px;
    white-space: pre-wrap; z-index: 9999; box-shadow: 0 4px 12px rgba(0,0,0,0.4);
  `;
  div.textContent = msg;
  document.body.appendChild(div);
  console.error('[Lotus]', msg);
}

// ====== 检查依赖是否加载成功 ======
function checkDeps() {
  const errors = [];
  if (typeof Terminal === 'undefined') errors.push('xterm.min.js 未加载（window.Terminal 缺失）');
  if (typeof FitAddon === 'undefined') errors.push('xterm-addon-fit 未加载（window.FitAddon 缺失）');
  if (typeof WebLinksAddon === 'undefined') errors.push('xterm-addon-web-links 未加载');
  return errors;
}

// ====== 全局状态 ======
const terminals = new Map(); // tabId -> { term, fit, pane, title, projectId }
let nextTabTitle = (id) => `lotus ${id}`;

// tab 运行时（agent / 忙碌状态 / 徽章）
// tabId -> { busy, cmd, isAgent }
const tabRuntime = new Map();

const AGENT_NAME_RE = /^(claude|codex|opencode|gemini|aider|pi|cursor-agent|cn)\b/i;
function isAgentCommand(text) {
  if (!text) return false;
  return AGENT_NAME_RE.test(String(text).trim());
}

// 每 project 记忆自己的 active tab（切换项目时恢复）
const activeTabByProject = new Map(); // projectId -> tabId
function getActiveTabId() {
  return activeTabByProject.get(_currentProjectId) ?? null;
}
function setActiveTabId(tabId) {
  if (_currentProjectId !== null) {
    activeTabByProject.set(_currentProjectId, tabId);
  }
}
// _currentProjectId 在文件下方定义（历史/书签缓存区），这里 forward declare
let _currentProjectId = null;

// ====== IPC 桥（__lotus 由 Rust 的 UserScript 最早注入，带缓冲队列）======
// app.js 只需注册 _handle（实际处理函数），队列里的消息会自动冲刷
window.__lotus = window.__lotus || { _queue: [], _ready: false };
window.__lotus._handle = (msg) => {
  console.log('[lotus] ← backend:', msg);
  try {
    handleServerMessage(msg);
  } catch (e) {
    console.error('处理服务器消息失败:', e, msg);
  }
};
// 冲刷 UserScript 时期缓冲的消息
window.__lotus._ready = true;
const _queued = window.__lotus._queue || [];
window.__lotus._queue = [];
setTimeout(() => {
  for (const q of _queued) {
    try { window.__lotus._handle(q); } catch (e) {}
  }
}, 50);

// 发送消息给 Rust 后端
function sendToBackend(msg) {
  const json = JSON.stringify(msg);
  console.log('[lotus] → backend:', json.substring(0, 80));
  window.ipc.postMessage(json);
}

// ====== 创建 xterm.js 终端实例 ======
// 注意：必须在 pane 可见（active）之后再 open + fit，否则容器尺寸为 0
function createTerminal(tabId, cols, rows, projectId) {
  // 幂等：如果该 tab 已存在，不重复创建（防止消息重复处理导致多个 xterm 叠加）
  if (terminals.has(tabId)) {
    console.warn('[lotus] tab', tabId, '已存在，跳过重复创建');
    return terminals.get(tabId);
  }
  try {
    // 移除启动诊断条
    const diag = document.getElementById('startup-diag');
    if (diag) diag.remove();

    const pane = document.createElement('div');
    pane.className = 'terminal-pane active';
    pane.dataset.tabId = tabId;
    pane.dataset.projectId = projectId;
    // 如果不是当前项目的 tab，先隐藏（切换项目时显示）
    if (_currentProjectId !== null && projectId !== _currentProjectId) {
      pane.classList.remove('active');
      pane.style.display = 'none';
    }
    document.getElementById('terminals').appendChild(pane);

    const term = new Terminal({
      cols: cols || 80,
      rows: rows || 24,
      // 字体优先级：JetBrains Mono（已装）→ DejaVu Sans Mono（系统自带）→ monospace
      // 必须明确列出系统已有的字体，避免回退到泛 monospace 触发 webkit2gtk 宽度 bug
      fontFamily: "'JetBrains Mono', 'DejaVu Sans Mono', 'Ubuntu Mono', 'Liberation Mono', monospace",
      fontSize: 14,
      letterSpacing: 0,  // 显式 0，避免任何额外字间距
      lineHeight: 1,
      cursorBlink: true,
      cursorStyle: 'bar',
      allowProposedApi: true,
      // 关键：让 xterm 用 DOM 渲染器（比 canvas 在 webkit2gtk 下更可靠）
      rendererType: 'dom',
      theme: {
        background: '#1e1b26',
        foreground: '#e6e1eb',
        cursor: '#e88da7',
        cursorAccent: '#1e1b26',
        selectionBackground: '#50465f',
        black: '#1e1b26',
        red: '#e06c75',
        green: '#98c379',
        yellow: '#e5c07b',
        blue: '#61afef',
        magenta: '#c678dd',
        cyan: '#56b6c2',
        white: '#e6e1eb',
        brightBlack: '#827a91',
        brightRed: '#e06c75',
        brightGreen: '#98c379',
        brightYellow: '#e5c07b',
        brightBlue: '#61afef',
        brightMagenta: '#c678dd',
        brightCyan: '#56b6c2',
        brightWhite: '#ffffff',
      },
    });

    const fit = new FitAddon.FitAddon();
    const webLinks = new WebLinksAddon.WebLinksAddon();
    term.loadAddon(fit);
    term.loadAddon(webLinks);

    term.open(pane);

    // 多次 fit 解决字体异步加载导致字符宽度测量不准的问题
    // 字体没加载完时 fit 会算错 cols，导致字符显示分散
    const doFit = () => {
      try {
        fit.fit();
        notifyResize(tabId, term, fit);
      } catch (e) {
        console.error('[lotus] fit 失败:', e);
      }
    };
    requestAnimationFrame(doFit);
    setTimeout(doFit, 100);   // 字体可能还在加载
    setTimeout(doFit, 500);   // 兜底
    setTimeout(() => term.focus(), 200);

    // 字体加载完后重新 fit（解决字符分散问题的关键）
    if (document.fonts && document.fonts.ready) {
      document.fonts.ready.then(doFit);
    }

    // 键盘输入：转发给 Rust
    term.onData((data) => {
      sendToBackend({
        type: 'input',
        tab_id: tabId,
        data: btoa(unescape(encodeURIComponent(data))),
      });
    });

    // 尺寸变化：通知 Rust 调整 PTY
    term.onResize(({ cols, rows }) => {
      sendToBackend({ type: 'resize', tab_id: tabId, cols, rows });
    });

    // 终端快捷键（GUI 风格）：
    //   Ctrl+C / Ctrl+Shift+C  → 复制选区
    //   Ctrl+V / Ctrl+Shift+V  → 粘贴
    //   Ctrl+Z                 → 中断当前进程（发送 SIGINT / \x03）
    term.attachCustomKeyEventHandler((ev) => {
      if (ev.type !== 'keydown' || !ev.ctrlKey || ev.altKey || ev.metaKey) return true;
      const key = (ev.key || '').toLowerCase();

      // Ctrl+C / Ctrl+Shift+C：复制（不向 PTY 发送 \x03）
      if (key === 'c') {
        const sel = term.getSelection();
        if (sel) {
          copyText(sel);
          try { term.clearSelection(); } catch (_) {}
        }
        return false;
      }

      // Ctrl+V / Ctrl+Shift+V：只标记并由唯一入口粘贴；return false 阻止 xterm 默认
      if (key === 'v') {
        markShortcutPaste();
        pasteToTab(tabId);
        return false;
      }

      // Ctrl+Z：中断（发送 ETX=\x03，即传统 Ctrl+C 的 SIGINT）
      // 不再发送传统 Ctrl+Z（\x1a / SIGTSTP）
      if (key === 'z' && !ev.shiftKey) {
        sendInterrupt(tabId);
        return false;
      }

      return true;
    });

    // 拦截 xterm 内部 textarea 的原生 paste：
    // - 始终 preventDefault，避免 WebKit/xterm 再插一份
    // - 若刚被 Ctrl+V 快捷键处理过，则不再粘贴（防止重复）
    const xtermTextarea = pane.querySelector('.xterm-helper-textarea');
    if (xtermTextarea) {
      xtermTextarea.addEventListener('paste', (ev) => {
        ev.preventDefault();
        ev.stopPropagation();
        if (wasShortcutPaste()) return;
        pasteToTab(tabId);
      }, true);
    }

    terminals.set(tabId, { term, fit, pane, title: nextTabTitle(tabId), projectId });
    return { term, fit, pane };
  } catch (e) {
    showErrorOverlay('createTerminal 失败: ' + e.message + '\n' + (e.stack || ''));
    return null;
  }
}

// 通知 Rust 当前终端尺寸
function notifyResize(tabId, term, fit) {
  sendToBackend({
    type: 'resize',
    tab_id: tabId,
    cols: term.cols,
    rows: term.rows,
  });
}

// ====== 处理 Rust → JS 消息 ======
function handleServerMessage(msg) {
  switch (msg.type) {
    case 'output': {
      const t = terminals.get(msg.tab_id);
      if (t) {
        // base64 → 字符串 → 写入 xterm
        const bytes = atob(msg.data);
        const decoded = decodeURIComponent(escape(bytes));
        t.term.write(decoded);
      }
      break;
    }
    case 'tab_created': {
      // 创建前端终端实例 + tab UI（带 project_id 归属）
      createTerminal(msg.tab_id, msg.cols, msg.rows, msg.project_id);
      addTabUI(msg.tab_id, msg.title, msg.project_id);
      // 只在 tab 属于当前项目时切换激活（避免后台项目的 tab 抢焦点）
      if (msg.project_id === _currentProjectId) {
        switchTab(msg.tab_id);
      }
      hideLoading();
      break;
    }
    case 'tab_closed': {
      closeTabUI(msg.tab_id);
      const t = terminals.get(msg.tab_id);
      if (t) {
        t.term.dispose();
        terminals.delete(msg.tab_id);
      }
      // 清理 activeTabByProject 里指向已关闭 tab 的记录
      for (const [pid, tid] of activeTabByProject.entries()) {
        if (tid === msg.tab_id) activeTabByProject.delete(pid);
      }
      // 如果关的是当前项目的 active tab，切到同项目剩下的第一个
      if (getActiveTabId() === null && _currentProjectId !== null) {
        const sameProject = [...terminals.values()]
          .filter((tt) => tt.projectId === _currentProjectId);
        if (sameProject.length > 0) {
          const firstTabId = [...terminals.entries()].find(
            ([, tt]) => tt.projectId === _currentProjectId
          )[0];
          switchTab(firstTabId);
        }
      }
      break;
    }
    case 'title_changed': {
      const t = terminals.get(msg.tab_id);
      if (t) t.title = msg.title;
      updateTabTitle(msg.tab_id, msg.title);
      if (isAgentCommand(msg.title)) {
        const rt = ensureTabRuntime(msg.tab_id);
        rt.isAgent = true;
        const tabEl = document.querySelector(`.tab[data-tab-id="${msg.tab_id}"]`);
        if (tabEl) tabEl.classList.add('has-agent');
      }
      break;
    }
    case 'theme': {
      applyTheme(msg.theme);
      // 同步更新所有已开终端的 xterm 配色
      applyXtermTheme(msg.theme);
      break;
    }
    // ===== 设置相关 =====
    case 'config': {
      fillSettingsForm(msg.config);
      break;
    }
    case 'themes_list': {
      fillSelect('setting-theme', msg.names, getCurrentConfig().theme);
      break;
    }
    case 'fonts_list': {
      fillSelect('setting-font', msg.names, getCurrentConfig().font);
      break;
    }
    case 'shells_list': {
      fillSelect('setting-shell', msg.names, getCurrentConfig().shell, true);
      break;
    }
    case 'config_saved': {
      const status = document.getElementById('save-status');
      if (status) {
        status.textContent = msg.ok ? '✓ 已保存' : '✗ 保存失败';
        status.className = 'save-status ' + (msg.ok ? 'success' : 'error');
        setTimeout(() => { status.textContent = ''; status.className = 'save-status'; }, 2500);
      }
      break;
    }
    // ===== 历史 / 书签 =====
    case 'history_list': {
      _historyCache = msg.entries;
      renderHistory(msg.entries);
      break;
    }
    case 'history_appended': {
      // 如果用户当前在历史视图或搜索为空，实时插入
      _historyCache.unshift(msg.entry);
      const searchEl = document.getElementById('history-search');
      const query = searchEl ? searchEl.value.trim() : '';
      // 仅在没有搜索过滤时实时刷新（避免破坏用户当前的搜索结果）
      if (!query) {
        renderHistory(_historyCache);
      }
      break;
    }
    case 'recents_list': {
      renderSidebarRecents(msg.paths);
      break;
    }
    case 'bookmarks_list': {
      _bookmarksCache = msg.entries;
      renderBookmarks(msg.entries);
      break;
    }
    case 'bookmark_added': {
      _bookmarksCache.push(msg.entry);
      renderBookmarks(_bookmarksCache);
      break;
    }
    case 'clipboard_text': {
      // 响应 clipboard_read：交给挂起的 Promise
      resolveClipboardRequest(msg.request_id, msg.text || '');
      break;
    }
    case 'agents_list': {
      renderAgentsList(msg.agents || []);
      break;
    }
    case 'command_started': {
      onCommandStarted(msg.tab_id, msg.cmd);
      break;
    }
    case 'command_finished': {
      onCommandFinished(msg.tab_id, msg.cmd, msg.code, msg.duration_ms || 0);
      break;
    }
    case 'bookmark_removed': {
      _bookmarksCache = _bookmarksCache.filter((b) => b.id !== msg.id);
      renderBookmarks(_bookmarksCache);
      break;
    }
    // ===== 项目 =====
    case 'projects_list': {
      _projectsCache = msg.entries;
      renderSidebarProjects(msg.entries);
      break;
    }
    case 'project_switched': {
      // 更新标题栏当前项目名
      const ws = document.getElementById('workspace-name');
      if (ws) ws.textContent = msg.name;
      _currentProjectId = msg.id;
      renderSidebarProjects(_projectsCache);  // 更新侧边栏高亮
      // 核心：按 project_id 过滤 tab 和 pane 的显隐（不杀任何 PTY）
      filterTabsByProject(msg.id);
      // 恢复该项目的 active tab（如果有）
      const restoreId = getActiveTabId();
      if (restoreId !== null && terminals.has(restoreId)) {
        switchTab(restoreId);
      }
      // 切回终端视图
      switchView('terminal');
      // 清空历史/书签缓存（下次打开面板会重新拉）
      _historyCache = [];
      _bookmarksCache = [];
      break;
    }
  }
}

// ====== 历史 / 书签 / 项目 数据缓存 ======
let _historyCache = [];
let _bookmarksCache = [];
let _projectsCache = [];
// _currentProjectId 已在文件顶部声明（与 activeTabByProject 一起）

// ====== 设置面板辅助函数 ======
// 当前配置缓存（由 config 消息填充，供其他逻辑查询）
let _currentConfig = { theme: 'lotus', shell: '', font: 'JetBrains Mono', font_size: 14, opacity: 1.0 };
function getCurrentConfig() { return _currentConfig; }

// 填充 select 下拉框
function fillSelect(id, options, selected, includeEmpty) {
  const sel = document.getElementById(id);
  if (!sel) return;
  sel.innerHTML = '';
  if (includeEmpty) {
    const opt = document.createElement('option');
    opt.value = '';
    opt.textContent = '（系统默认）';
    sel.appendChild(opt);
  }
  for (const name of options) {
    const opt = document.createElement('option');
    opt.value = name;
    opt.textContent = name;
    if (name === selected) opt.selected = true;
    sel.appendChild(opt);
  }
}

// 用配置数据填充设置表单
function fillSettingsForm(cfg) {
  _currentConfig = cfg;
  const setVal = (id, v) => { const el = document.getElementById(id); if (el) el.value = v; };
  setVal('setting-theme', cfg.theme);
  setVal('setting-font', cfg.font);
  setVal('setting-font-size', cfg.font_size);
  setVal('setting-shell', cfg.shell);
  const opacityEl = document.getElementById('setting-opacity');
  if (opacityEl) opacityEl.value = Math.round(cfg.opacity * 100);
  updateOpacityDisplay();
  updateFontPreview();
}

// 同步 xterm 配色（主题切换时调用）
function applyXtermTheme(theme) {
  terminals.forEach((t) => {
    try {
      t.term.setOption('theme', {
        background: theme.bg,
        foreground: theme.fg,
        cursor: theme.accent,
        cursorAccent: theme.bg,
        selectionBackground: theme.block_border,
      });
    } catch (e) { console.warn('setOption theme 失败:', e); }
  });
}

// ====== Tab UI 管理 ======
function addTabUI(tabId, title, projectId) {
  const tabBar = document.getElementById('tabs');
  const tab = document.createElement('div');
  tab.className = 'tab';
  tab.dataset.tabId = tabId;
  tab.dataset.projectId = projectId;
  // 非当前项目的 tab 隐藏（不杀，仅 display:none）
  if (_currentProjectId !== null && projectId !== _currentProjectId) {
    tab.style.display = 'none';
  }
  tab.innerHTML = `
    <span class="tab-badge" title=""></span>
    <span class="tab-title">${escapeHtml(title || nextTabTitle(tabId))}</span>
    <button class="tab-close" title="关闭">×</button>
  `;
  // 初始化 tab 运行时状态
  if (!tabRuntime.has(tabId)) {
    tabRuntime.set(tabId, { busy: false, cmd: '', isAgent: isAgentCommand(title) });
  }
  if (isAgentCommand(title)) tab.classList.add('has-agent');
  // 点击 tab 切换
  tab.addEventListener('click', (e) => {
    if (e.target.classList.contains('tab-close')) {
      e.stopPropagation();
      closeTab(tabId);
    } else {
      switchTab(tabId);
    }
  });
  tabBar.appendChild(tab);
}

function closeTabUI(tabId) {
  const tab = document.querySelector(`.tab[data-tab-id="${tabId}"]`);
  if (tab) tab.remove();
  tabRuntime.delete(tabId);
}

function updateTabTitle(tabId, title) {
  const tab = document.querySelector(`.tab[data-tab-id="${tabId}"] .tab-title`);
  if (tab) tab.textContent = title;
}

function switchTab(tabId) {
  setActiveTabId(tabId);
  // 更新当前项目可见 tab 的高亮
  document.querySelectorAll('.tab').forEach((t) => {
    t.classList.toggle('active', t.dataset.tabId == tabId);
  });
  // 聚焦 tab 时清除 done/error 徽章（保留 busy）
  clearTabDoneBadge(tabId);
  // 在当前项目的 tab 之间切换 pane 显示
  terminals.forEach((t, id) => {
    // 只处理当前项目的 tab（其他项目的 pane 保持隐藏）
    if (_currentProjectId !== null && t.projectId !== _currentProjectId) return;
    const isActive = id === tabId;
    t.pane.classList.toggle('active', isActive);
    if (isActive) {
      // 切换后重新 fit 并聚焦
      setTimeout(() => {
        try { t.fit.fit(); } catch (e) {}
        t.term.focus();
        notifyResize(tabId, t.term, t.fit);
      }, 10);
    }
  });
}

// 按项目过滤 tab/pane 显隐（切换项目时调用，不杀任何 PTY）
function filterTabsByProject(projectId) {
  // 过滤 tab 栏的 .tab 元素
  document.querySelectorAll('.tab').forEach((tab) => {
    const tid = parseInt(tab.dataset.projectId);
    tab.style.display = (tid === projectId) ? '' : 'none';
  });
  // 过滤终端 pane
  terminals.forEach((t) => {
    const isCurrent = t.projectId === projectId;
    t.pane.style.display = isCurrent ? '' : 'none';
    if (!isCurrent) t.pane.classList.remove('active');
  });
}

function closeTab(tabId) {
  sendToBackend({ type: 'close_tab', tab_id: tabId });
}

function newTab() {
  sendToBackend({ type: 'new_tab' });
}

// ====== 主题应用 ======
function applyTheme(theme) {
  const root = document.documentElement;
  if (theme.bg) root.style.setProperty('--bg', theme.bg);
  if (theme.fg) root.style.setProperty('--fg', theme.fg);
  if (theme.accent) root.style.setProperty('--accent', theme.accent);
  if (theme.muted) root.style.setProperty('--muted', theme.muted);
  if (theme.success) root.style.setProperty('--success', theme.success);
  if (theme.error) root.style.setProperty('--error', theme.error);
  if (theme.block_border) root.style.setProperty('--block-border', theme.block_border);
  if (theme.title_bg) root.style.setProperty('--title-bg', theme.title_bg);
  if (theme.sidebar_bg) root.style.setProperty('--sidebar-bg', theme.sidebar_bg);
  if (theme.tab_bg) root.style.setProperty('--tab-bg', theme.tab_bg);
}

// ====== 加载提示 ======
function hideLoading() {
  const loading = document.getElementById('loading');
  if (loading) {
    loading.classList.add('hidden');
    setTimeout(() => loading.remove(), 400);
  }
}

// ====== 工具函数 ======
function escapeHtml(s) {
  const div = document.createElement('div');
  div.textContent = s;
  return div.innerHTML;
}

// ====== 窗口控制按钮 + UI 事件绑定 ======
// 用 try/catch 包起来，防止单个绑定失败导致后续全部不执行
function bindUIEvents() {
  const bind = (id, event, handler) => {
    try {
      const el = document.getElementById(id);
      if (el) el.addEventListener(event, handler);
      else console.warn('[lotus] UI 元素不存在:', id);
    } catch (e) { console.error('[lotus] 绑定', id, '失败:', e); }
  };
  bind('btn-close', 'click', () => sendToBackend({ type: 'quit' }));
  bind('btn-min', 'click', () => {});
  bind('btn-max', 'click', () => {});
  bind('new-tab-btn', 'click', newTab);

  // 侧边栏导航：根据 data-view 切换视图
  document.querySelectorAll('.nav-item').forEach((item) => {
    try {
      item.addEventListener('click', () => switchView(item.dataset.view));
    } catch (e) { console.error('[lotus] 绑定 nav-item 失败:', e); }
  });
}

// ====== 视图切换：终端 / Agents / 设置 ======
function switchView(view) {
  console.log('[lotus] 切换视图:', view);
  // 切换侧边栏高亮
  document.querySelectorAll('.nav-item').forEach((n) => {
    n.classList.toggle('active', n.dataset.view === view);
  });
  // 隐藏所有视图，显示目标视图
  const allViews = ['content', 'view-settings', 'view-history', 'view-bookmarks', 'view-agents'];
  allViews.forEach((id) => {
    const el = document.getElementById(id);
    if (el) {
      // 终端视图（#content）以外的才加 hidden（terminal 是默认视图）
      if (id === 'content') {
        el.classList.toggle('hidden', view !== 'terminal');
      } else if (id === 'view-settings') {
        el.classList.toggle('hidden', view !== 'settings');
      } else if (id === 'view-history') {
        el.classList.toggle('hidden', view !== 'history');
      } else if (id === 'view-bookmarks') {
        el.classList.toggle('hidden', view !== 'bookmarks');
      } else if (id === 'view-agents') {
        el.classList.toggle('hidden', view !== 'agents');
      }
    }
  });
  // 视图特定的初始化动作
  if (view === 'settings') {
    sendToBackend({ type: 'get_config' });
  } else if (view === 'history') {
    sendToBackend({ type: 'get_history' });
  } else if (view === 'bookmarks') {
    sendToBackend({ type: 'get_bookmarks' });
  } else if (view === 'agents') {
    sendToBackend({ type: 'get_agents' });
  } else if (view === 'terminal') {
    // 切回终端时重新 fit
    setTimeout(() => {
      terminals.forEach((t) => { try { t.fit.fit(); } catch (e) {} });
    }, 50);
  }
}
// 等 DOM 就绪后绑定
if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', bindUIEvents);
} else {
  bindUIEvents();
}

// ====== 全局快捷键 ======
document.addEventListener('keydown', (e) => {
  // F12 或 Ctrl+Shift+I: 打开 devtools（诊断白屏用）
  if (e.key === 'F12' || (e.ctrlKey && e.shiftKey && (e.key === 'I' || e.key === 'i'))) {
    e.preventDefault();
    // 通知后端打开 devtools（如果支持）
    sendToBackend({ type: 'devtools' });
    return;
  }
  // Ctrl+T: 新 tab
  if (e.ctrlKey && e.key === 't' && !e.shiftKey) {
    e.preventDefault();
    newTab();
    return;
  }
  // Ctrl+W: 关闭当前 tab
  if (e.ctrlKey && e.key === 'w' && !e.shiftKey) {
    e.preventDefault();
    const aid = getActiveTabId();
    if (aid !== null) closeTab(aid);
    return;
  }
  // Ctrl+Q: 退出
  if (e.ctrlKey && e.key === 'q') {
    e.preventDefault();
    sendToBackend({ type: 'quit' });
    return;
  }
  // Ctrl+数字: 切换 tab
  if (e.ctrlKey && /^[1-9]$/.test(e.key)) {
    e.preventDefault();
    const idx = parseInt(e.key) - 1;
    const ids = [...terminals.keys()];
    if (ids[idx]) switchTab(ids[idx]);
    return;
  }
  // 终端快捷键兜底：仅在「焦点不在 xterm / 输入框」时处理
  // xterm 内的 Ctrl+C/V/Z 已由 attachCustomKeyEventHandler 唯一处理
  if (
    e.ctrlKey && !e.altKey && !e.metaKey &&
    !isUiEditableTarget(e.target) &&
    !isInTerminalUi(e.target)
  ) {
    const key = (e.key || '').toLowerCase();
    const aid = getActiveTabId();
    if (aid === null) return;

    if (key === 'c') {
      e.preventDefault();
      const entry = terminals.get(aid);
      if (entry && entry.term) {
        const sel = entry.term.getSelection();
        if (sel) {
          copyText(sel);
          try { entry.term.clearSelection(); } catch (_) {}
        }
      }
      return;
    }

    if (key === 'v') {
      e.preventDefault();
      markShortcutPaste();
      pasteToTab(aid);
      return;
    }

    if (key === 'z' && !e.shiftKey) {
      e.preventDefault();
      sendInterrupt(aid);
      return;
    }
  }
});

// ====== 窗口尺寸变化时重新 fit 所有终端 ======
window.addEventListener('resize', () => {
  terminals.forEach((t) => {
    try { t.fit.fit(); } catch (e) {}
  });
});

// ====== 启动：检查依赖 + 通知后端前端已就绪 ======
window.addEventListener('DOMContentLoaded', () => {
  // 立即隐藏 loading（让我们能看到后面的内容/诊断条）
  hideLoading();

  // 诊断条：在 terminals 容器里放可见的诊断条
  const terminalsDiv = document.getElementById('terminals');
  const diag = document.createElement('div');
  diag.id = 'startup-diag';
  diag.style.cssText = 'padding:20px;color:#e88da7;font-family:monospace;font-size:14px;white-space:pre-wrap;background:#1e1b26;height:100%;';
  diag.textContent = 'Lotus 启动中...\n检查前端依赖...\n';
  if (terminalsDiv) terminalsDiv.appendChild(diag);

  function logDiag(msg) {
    diag.textContent += msg + '\n';
    console.log('[lotus]', msg);
  }

  // 1. 检查 xterm.js 及 addon
  const depErrors = checkDeps();
  if (depErrors.length > 0) {
    logDiag('❌ ' + depErrors.join('\n❌ '));
    showErrorOverlay('前端依赖加载失败：\n  ' + depErrors.join('\n  '));
    return;
  }
  logDiag('✓ 依赖检查通过');

  // 2. 检查 IPC 桥
  if (typeof window.ipc === 'undefined' || !window.ipc.postMessage) {
    logDiag('❌ IPC 桥未注入（window.ipc.postMessage 不存在）');
    logDiag('⚠ 这说明 webkit2gtk 的 register_script_message_handler 没生效');
    showErrorOverlay('IPC 桥未注入');
    return;
  }
  logDiag('✓ IPC 桥就绪');

  // 3. 容器尺寸
  if (terminalsDiv) {
    logDiag(`📐 terminals 容器: ${terminalsDiv.offsetWidth}x${terminalsDiv.offsetHeight}px`);
  }

  logDiag('发送 ready，等待后端创建首个 tab...');

  // 通知后端创建首个 tab
  setTimeout(() => {
    try {
      // 标记 ready，并冲刷之前缓冲的消息
      window.__lotus._ready = true;
      const queued = window.__lotus._queue;
      window.__lotus._queue = [];
      sendToBackend({ type: 'ready' });
      logDiag('✓ ready 已发送');
      // 处理在 ready 之前到达的消息
      for (const q of queued) {
        try { handleServerMessage(q); } catch (e) {}
      }
    } catch (e) {
      logDiag('❌ 发送 ready 失败: ' + e.message);
      showErrorOverlay('发送 ready 失败: ' + e.message);
    }
  }, 100);
});

// ====== 设置面板交互（实时预览 + 保存 + 重置）======
function setupSettingsPanel() {
  const bind = (id, event, handler) => {
    const el = document.getElementById(id);
    if (el) el.addEventListener(event, handler);
    else console.warn('[lotus] 设置元素不存在:', id);
  };

  // 主题切换：实时预览
  bind('setting-theme', 'change', (e) => {
    const name = e.target.value;
    sendToBackend({ type: 'set_theme', name });
    _currentConfig.theme = name;
    showStatus('主题预览中（点保存以持久化）');
  });

  // 窗口透明度：拖动实时预览
  const opacityEl = document.getElementById('setting-opacity');
  if (opacityEl) {
    opacityEl.addEventListener('input', () => {
      updateOpacityDisplay();
      applyOpacityPreview();
    });
  }

  // 字体切换：实时应用到 xterm + 预览
  bind('setting-font', 'change', (e) => {
    _currentConfig.font = e.target.value;
    applyFontToTerminals();
    updateFontPreview();
  });

  // 字号 ± 按钮和直接输入
  bind('font-size-minus', 'click', () => {
    const el = document.getElementById('setting-font-size');
    const v = Math.max(10, parseInt(el.value || '14') - 1);
    el.value = v;
    _currentConfig.font_size = v;
    applyFontToTerminals();
    updateFontPreview();
  });
  bind('font-size-plus', 'click', () => {
    const el = document.getElementById('setting-font-size');
    const v = Math.min(32, parseInt(el.value || '14') + 1);
    el.value = v;
    _currentConfig.font_size = v;
    applyFontToTerminals();
    updateFontPreview();
  });
  bind('setting-font-size', 'change', (e) => {
    const v = Math.min(32, Math.max(10, parseInt(e.target.value) || 14));
    e.target.value = v;
    _currentConfig.font_size = v;
    applyFontToTerminals();
    updateFontPreview();
  });

  // 保存按钮
  bind('save-btn', 'click', () => {
    const config = collectFormConfig();
    sendToBackend({ type: 'save_config', config });
  });

  // 重置按钮
  bind('reset-btn', 'click', () => {
    const defaults = { theme: 'lotus', shell: '', font: 'JetBrains Mono', font_size: 14, opacity: 1.0 };
    fillSettingsForm(defaults);
    sendToBackend({ type: 'set_theme', name: 'lotus' });
    applyOpacityPreview();
    applyFontToTerminals();
    showStatus('已重置为默认值（点保存以持久化）');
  });
}

// 显示透明度数值
function updateOpacityDisplay() {
  const el = document.getElementById('setting-opacity');
  const valEl = document.getElementById('setting-opacity-value');
  if (el && valEl) valEl.textContent = el.value + '%';
}

// 应用透明度预览（通过 --opacity CSS 变量控制终端区透明度）
function applyOpacityPreview() {
  const el = document.getElementById('setting-opacity');
  if (!el) return;
  const opacity = parseInt(el.value) / 100;
  _currentConfig.opacity = opacity;
  // 设置 CSS 变量，让 .terminal-pane 和 xterm 背景按透明度渲染
  document.documentElement.style.setProperty('--opacity', opacity);
  // 给当前激活的终端 pane 应用半透明背景
  terminals.forEach((t) => {
    try {
      t.pane.style.backgroundColor = `rgba(30, 27, 38, ${opacity})`;
      // xterm 的背景通过 setOption 更稳
      t.term.setOption('theme', {
        ...t.term.getOption('theme'),
        background: `rgba(30, 27, 38, ${opacity})`,
      });
    } catch (e) {}
  });
}

// 应用字体到所有终端
function applyFontToTerminals() {
  terminals.forEach((t) => {
    try {
      t.term.setOption('fontFamily', _currentConfig.font + ", 'DejaVu Sans Mono', monospace");
      t.term.setOption('fontSize', _currentConfig.font_size);
      // 字体变化后必须重新 fit
      setTimeout(() => { try { t.fit.fit(); } catch (e) {} }, 50);
    } catch (e) { console.warn('字体应用失败:', e); }
  });
}

// 更新字体预览区
function updateFontPreview() {
  const preview = document.getElementById('font-preview');
  if (preview) {
    preview.style.fontFamily = _currentConfig.font + ", monospace";
    preview.style.fontSize = _currentConfig.font_size + 'px';
  }
}

// 收集表单配置
function collectFormConfig() {
  const getVal = (id) => { const el = document.getElementById(id); return el ? el.value : ''; };
  const opacityEl = document.getElementById('setting-opacity');
  return {
    theme: getVal('setting-theme') || 'lotus',
    shell: getVal('setting-shell') || '',
    font: getVal('setting-font') || 'JetBrains Mono',
    font_size: parseInt(getVal('setting-font-size')) || 14,
    opacity: opacityEl ? parseInt(opacityEl.value) / 100 : 1.0,
  };
}

// 状态栏临时提示
function showStatus(msg) {
  const status = document.getElementById('save-status');
  if (status) {
    status.textContent = msg;
    status.className = 'save-status';
    if (!msg.includes('失败') && !msg.includes('错误')) {
      setTimeout(() => { if (status.textContent === msg) { status.textContent = ''; } }, 2500);
    }
  }
}

// DOM 就绪后初始化设置面板
if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', setupSettingsPanel);
} else {
  setupSettingsPanel();
}

// ====== 历史 / 书签 面板渲染与交互 ======

// 相对时间格式化（"2分钟前" / "昨天" / "12-25 14:30"）
function formatRelativeTime(ts) {
  const now = Math.floor(Date.now() / 1000);
  const diff = now - ts;
  if (diff < 60) return '刚刚';
  if (diff < 3600) return Math.floor(diff / 60) + ' 分钟前';
  if (diff < 86400) return Math.floor(diff / 3600) + ' 小时前';
  if (diff < 86400 * 2) return '昨天';
  if (diff < 86400 * 7) return Math.floor(diff / 86400) + ' 天前';
  const d = new Date(ts * 1000);
  return (d.getMonth() + 1) + '-' + d.getDate() + ' ' + d.getHours() + ':' + String(d.getMinutes()).padStart(2, '0');
}

// 把路径里的 $HOME 替换为 ~（更短，易读）
function shortenPath(p) {
  const home = (window.__lotus && window.__lotus.home) || '';
  if (home && p.startsWith(home)) return '~' + p.slice(home.length);
  return p;
}

// 渲染历史列表
function renderHistory(entries) {
  const list = document.getElementById('history-list');
  const countEl = document.getElementById('history-count');
  if (countEl) countEl.textContent = '共 ' + entries.length + ' 条';
  if (!list) return;
  if (entries.length === 0) {
    list.innerHTML = '<div class="empty-state">暂无历史记录，执行命令后会显示在这里</div>';
    return;
  }
  list.innerHTML = '';
  for (const e of entries) {
    const item = document.createElement('div');
    item.className = 'list-item';
    const isError = e.code !== 0;
    item.innerHTML =
      '<span class="list-item-icon">$</span>' +
      '<div class="list-item-main">' +
        '<div class="list-item-title"></div>' +
        '<div class="list-item-sub"></div>' +
      '</div>' +
      '<span class="list-item-meta ' + (isError ? 'error' : 'success') + '">' +
        (isError ? '✗ ' : '✓ ') + e.code + '<br>' + formatRelativeTime(e.ts) +
      '</span>';
    // 用 textContent 避免 XSS
    item.querySelector('.list-item-title').textContent = e.cmd;
    item.querySelector('.list-item-sub').textContent = shortenPath(e.cwd);
    // 点击重新执行
    item.addEventListener('click', () => {
      runInActiveTab(e.cmd);
    });
    list.appendChild(item);
  }
}

// 渲染书签列表
function renderBookmarks(entries) {
  const list = document.getElementById('bookmarks-list');
  if (!list) return;
  if (entries.length === 0) {
    list.innerHTML = '<div class="empty-state">暂无书签，使用下方表单添加</div>';
    return;
  }
  list.innerHTML = '';
  for (const b of entries) {
    const item = document.createElement('div');
    item.className = 'list-item';
    item.innerHTML =
      '<span class="list-item-icon">📁</span>' +
      '<div class="list-item-main">' +
        '<div class="list-item-title"></div>' +
        '<div class="list-item-sub"></div>' +
      '</div>' +
      '<button class="list-item-delete" title="删除">×</button>';
    item.querySelector('.list-item-title').textContent = b.name;
    item.querySelector('.list-item-sub').textContent = shortenPath(b.path);
    // 点击主区域 cd 过去
    item.querySelector('.list-item-main').addEventListener('click', () => {
      cdToBookmark(b.path);
    });
    item.querySelector('.list-item-icon').addEventListener('click', () => cdToBookmark(b.path));
    // 删除按钮
    item.querySelector('.list-item-delete').addEventListener('click', (ev) => {
      ev.stopPropagation();
      sendToBackend({ type: 'remove_bookmark', id: b.id });
    });
    list.appendChild(item);
  }
}

// 渲染侧边栏最近目录
function renderSidebarRecents(paths) {
  const container = document.getElementById('sidebar-recents');
  if (!container) return;
  if (paths.length === 0) {
    container.innerHTML = '<div class="section-item empty-hint">暂无记录</div>';
    return;
  }
  container.innerHTML = '';
  for (const p of paths) {
    const item = document.createElement('div');
    item.className = 'section-item';
    item.textContent = shortenPath(p);
    item.title = p;
    item.dataset.ctx = 'recent';
    item.dataset.path = p;
    item.addEventListener('click', () => cdToBookmark(p));
    container.appendChild(item);
  }
}

// 在当前活动 tab 执行命令（切回终端 + 写 PTY）
function runInActiveTab(command) {
  const aid = getActiveTabId();
  if (aid === null) {
    console.warn('[lotus] 没有活动 tab，无法执行命令');
    return;
  }
  sendToBackend({ type: 'run_in_tab', tab_id: aid, command });
  switchView('terminal');
}

// cd 到指定路径（封装的 runInActiveTab）
function cdToBookmark(path) {
  runInActiveTab('cd ' + path);
}

// ====== 绑定历史/书签面板的事件 ======
function setupHistoryBookmarks() {
  const bind = (id, event, handler) => {
    const el = document.getElementById(id);
    if (el) el.addEventListener(event, handler);
    else console.warn('[lotus] 元素不存在:', id);
  };
  // 历史搜索框：实时本地过滤
  bind('history-search', 'input', (e) => {
    const q = e.target.value.trim().toLowerCase();
    const filtered = q
      ? _historyCache.filter((h) => h.cmd.toLowerCase().includes(q))
      : _historyCache;
    renderHistory(filtered);
  });
  // 清空历史
  bind('clear-history-btn', 'click', () => {
    if (confirm('确定清空所有历史记录吗？此操作不可撤销。')) {
      sendToBackend({ type: 'clear_history' });
    }
  });
  // 添加书签
  bind('add-bookmark-btn', 'click', () => {
    const nameEl = document.getElementById('bookmark-name');
    const pathEl = document.getElementById('bookmark-path');
    const name = nameEl ? nameEl.value.trim() : '';
    const path = pathEl ? pathEl.value.trim() : '';
    if (!path) {
      alert('请填写路径');
      return;
    }
    sendToBackend({ type: 'add_bookmark', name: name || path, path });
    if (nameEl) nameEl.value = '';
    if (pathEl) pathEl.value = '';
  });
  // 回车也能添加书签
  bind('bookmark-path', 'keydown', (e) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      document.getElementById('add-bookmark-btn').click();
    }
  });
}
if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', setupHistoryBookmarks);
} else {
  setupHistoryBookmarks();
}

// ====== 项目（Workspace）渲染与交互 ======

// 渲染侧边栏项目列表
function renderSidebarProjects(entries) {
  const container = document.getElementById('sidebar-projects');
  if (!container) return;
  if (entries.length === 0) {
    container.innerHTML = '<div class="section-item empty-hint">无项目</div>';
    return;
  }
  container.innerHTML = '';
  for (const p of entries) {
    const item = document.createElement('div');
    item.className = 'project-item' + (p.id === _currentProjectId ? ' active' : '');
    item.dataset.ctx = 'project';
    item.dataset.id = String(p.id);
    item.dataset.name = p.name;
    item.dataset.cwd = p.cwd;
    item.innerHTML =
      '<span class="project-dot"></span>' +
      '<span class="project-name"></span>' +
      '<button class="project-delete" title="删除项目">×</button>';
    item.querySelector('.project-name').textContent = p.name;
    item.title = p.cwd;
    // 点击切换项目
    item.addEventListener('click', (ev) => {
      if (ev.target.classList.contains('project-delete')) return;
      if (p.id === _currentProjectId) return;  // 已是当前项目
      sendToBackend({ type: 'switch_project', id: p.id });
    });
    // 删除按钮
    item.querySelector('.project-delete').addEventListener('click', (ev) => {
      ev.stopPropagation();
      if (confirm('确定删除项目「' + p.name + '」吗？该项目的历史和书签将一并删除。')) {
        sendToBackend({ type: 'delete_project', id: p.id });
      }
    });
    container.appendChild(item);
  }
}

// 绑定项目相关交互（新建项目 modal、+ 按钮）
function setupProjects() {
  const bind = (id, event, handler) => {
    const el = document.getElementById(id);
    if (el) el.addEventListener(event, handler);
    else console.warn('[lotus] 项目元素不存在:', id);
  };

  // 启动时拉取项目列表
  sendToBackend({ type: 'get_projects' });

  // "+" 按钮打开 modal
  bind('add-project-btn', 'click', () => openNewProjectModal());

  // 取消按钮 / 遮罩点击关闭 modal
  bind('new-project-cancel', 'click', closeNewProjectModal);
  const overlay = document.querySelector('#new-project-modal .modal-overlay');
  if (overlay) overlay.addEventListener('click', closeNewProjectModal);

  // 创建按钮
  bind('new-project-create', 'click', () => {
    const name = (document.getElementById('new-project-name') || {}).value || '';
    const cwd = (document.getElementById('new-project-cwd') || {}).value || '';
    if (!cwd.trim()) {
      alert('请填写工作目录');
      return;
    }
    sendToBackend({
      type: 'new_project',
      name: name.trim() || cwd.trim(),
      cwd: cwd.trim(),
    });
    closeNewProjectModal();
  });

  // modal 内回车提交
  bind('new-project-cwd', 'keydown', (e) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      document.getElementById('new-project-create').click();
    }
  });
  bind('new-project-name', 'keydown', (e) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      document.getElementById('new-project-cwd').focus();
    }
  });

  // Esc 关闭 modal
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') {
      const modal = document.getElementById('new-project-modal');
      if (modal && !modal.classList.contains('hidden')) {
        closeNewProjectModal();
      }
    }
  });
}

function closeNewProjectModal() {
  const modal = document.getElementById('new-project-modal');
  if (modal) modal.classList.add('hidden');
}

// 打开新建项目 modal，可预填路径
function openNewProjectModal(prefill) {
  const modal = document.getElementById('new-project-modal');
  if (!modal) return;
  modal.classList.remove('hidden');
  const nameInput = document.getElementById('new-project-name');
  const cwdInput = document.getElementById('new-project-cwd');
  if (nameInput) {
    nameInput.value = (prefill && prefill.name) || '';
  }
  if (cwdInput) {
    cwdInput.value = (prefill && prefill.cwd) || '';
  }
  setTimeout(() => {
    if (nameInput && !nameInput.value) nameInput.focus();
    else if (cwdInput) cwdInput.focus();
  }, 50);
}

// 初始化项目交互
if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', setupProjects);
} else {
  setupProjects();
}

// ====== 自定义右键菜单（OTTY 风格，屏蔽浏览器默认菜单） ======

let _renameHandler = null;

// 剪贴板请求回调（request_id -> resolve）
const _clipboardWaiters = new Map();
let _clipboardReqSeq = 1;

function resolveClipboardRequest(requestId, text) {
  const waiter = _clipboardWaiters.get(requestId);
  if (!waiter) return;
  _clipboardWaiters.delete(requestId);
  try { waiter(text); } catch (e) { console.warn('[lotus] clipboard waiter 失败:', e); }
}

/** 通过 GTK 后端读取系统剪贴板 */
function readClipboardText() {
  return new Promise((resolve) => {
    const requestId = _clipboardReqSeq++;
    _clipboardWaiters.set(requestId, resolve);
    sendToBackend({ type: 'clipboard_read', request_id: requestId });
    // 超时兜底，避免 Promise 永久挂起
    setTimeout(() => {
      if (_clipboardWaiters.has(requestId)) {
        _clipboardWaiters.delete(requestId);
        resolve('');
      }
    }, 2000);
  });
}

/** 通过 GTK 后端写入系统剪贴板 */
function copyText(text) {
  if (!text) return;
  sendToBackend({ type: 'clipboard_write', text: String(text) });
}

// ---- 粘贴防重 ----
// Ctrl+V 会同时触发 keydown + paste 事件；用时间窗合并为一次
let _pasteLockUntil = 0;
let _shortcutPasteUntil = 0;

function markShortcutPaste() {
  _shortcutPasteUntil = Date.now() + 200;
}

function wasShortcutPaste() {
  return Date.now() < _shortcutPasteUntil;
}

/** 把系统剪贴板内容粘贴进指定终端 tab（带防抖，保证只写一次） */
function pasteToTab(tabId) {
  if (tabId === null || tabId === undefined) return;
  const now = Date.now();
  if (now < _pasteLockUntil) {
    console.log('[lotus] 忽略重复粘贴');
    return;
  }
  _pasteLockUntil = now + 200;
  sendToBackend({ type: 'paste_to_tab', tab_id: tabId });
}

/** 向指定 tab 发送中断信号（SIGINT = \x03，传统 Ctrl+C） */
function sendInterrupt(tabId) {
  if (tabId === null || tabId === undefined) return;
  sendToBackend({
    type: 'input',
    tab_id: tabId,
    data: btoa('\x03'),
  });
}

/** 是否为 UI 输入控件（排除 xterm 内部隐藏 textarea） */
function isUiEditableTarget(el) {
  if (!el || !el.tagName) return false;
  const tag = el.tagName.toLowerCase();
  if (tag === 'input') return true;
  if (tag === 'textarea' && !el.classList.contains('xterm-helper-textarea')) return true;
  if (el.isContentEditable) return true;
  return false;
}

/** 事件目标是否位于终端 UI 内（由 xterm 快捷键处理器负责） */
function isInTerminalUi(el) {
  return !!(el && el.closest && el.closest('.terminal-pane, .xterm, #terminals'));
}

function hideContextMenu() {
  const menu = document.getElementById('ctx-menu');
  if (menu) {
    menu.classList.add('hidden');
    menu.innerHTML = '';
  }
}

function positionContextMenu(menu, x, y) {
  menu.classList.remove('hidden');
  // 先放到视口内再量尺寸，避免贴边溢出
  menu.style.left = '0px';
  menu.style.top = '0px';
  const rect = menu.getBoundingClientRect();
  const pad = 6;
  let left = x;
  let top = y;
  if (left + rect.width > window.innerWidth - pad) {
    left = Math.max(pad, window.innerWidth - rect.width - pad);
  }
  if (top + rect.height > window.innerHeight - pad) {
    top = Math.max(pad, window.innerHeight - rect.height - pad);
  }
  menu.style.left = left + 'px';
  menu.style.top = top + 'px';
}

/**
 * items: [{ label, action?, shortcut?, danger?, disabled?, sep? }]
 */
function showContextMenu(x, y, items) {
  const menu = document.getElementById('ctx-menu');
  if (!menu) return;
  menu.innerHTML = '';

  for (const it of items) {
    if (it.sep) {
      const sep = document.createElement('div');
      sep.className = 'ctx-menu-sep';
      menu.appendChild(sep);
      continue;
    }
    const btn = document.createElement('button');
    btn.type = 'button';
    btn.className = 'ctx-menu-item' + (it.danger ? ' danger' : '');
    btn.disabled = !!it.disabled;
    btn.setAttribute('role', 'menuitem');

    const label = document.createElement('span');
    label.textContent = it.label;
    btn.appendChild(label);

    if (it.shortcut) {
      const sc = document.createElement('span');
      sc.className = 'ctx-shortcut';
      sc.textContent = it.shortcut;
      btn.appendChild(sc);
    }

    if (!it.disabled && typeof it.action === 'function') {
      btn.addEventListener('click', (ev) => {
        ev.preventDefault();
        ev.stopPropagation();
        hideContextMenu();
        try { it.action(); } catch (e) { console.error('[lotus] 菜单动作失败:', e); }
      });
    }
    menu.appendChild(btn);
  }

  positionContextMenu(menu, x, y);

  // 打开后聚焦第一项，便于键盘操作
  const first = menu.querySelector('.ctx-menu-item:not(:disabled)');
  if (first) first.focus();
}

function openRenameModal(opts) {
  const modal = document.getElementById('rename-modal');
  if (!modal) return;
  const titleEl = document.getElementById('rename-modal-title');
  const labelEl = document.getElementById('rename-modal-label');
  const input = document.getElementById('rename-modal-input');
  if (titleEl) titleEl.textContent = opts.title || '重命名';
  if (labelEl) labelEl.textContent = opts.label || '名称';
  if (input) {
    input.value = opts.value || '';
    input.placeholder = opts.placeholder || '输入新名称';
  }
  _renameHandler = typeof opts.onConfirm === 'function' ? opts.onConfirm : null;
  modal.classList.remove('hidden');
  setTimeout(() => {
    if (input) {
      input.focus();
      input.select();
    }
  }, 50);
}

function closeRenameModal() {
  const modal = document.getElementById('rename-modal');
  if (modal) modal.classList.add('hidden');
  _renameHandler = null;
}

function confirmRenameModal() {
  const input = document.getElementById('rename-modal-input');
  const value = input ? input.value.trim() : '';
  if (!value) {
    alert('名称不能为空');
    return;
  }
  const handler = _renameHandler;
  closeRenameModal();
  if (handler) handler(value);
}

function setupRenameModal() {
  const bind = (id, event, handler) => {
    const el = document.getElementById(id);
    if (el) el.addEventListener(event, handler);
  };
  bind('rename-modal-cancel', 'click', closeRenameModal);
  bind('rename-modal-confirm', 'click', confirmRenameModal);
  const overlay = document.querySelector('#rename-modal .modal-overlay');
  if (overlay) overlay.addEventListener('click', closeRenameModal);
  bind('rename-modal-input', 'keydown', (e) => {
    if (e.key === 'Enter') {
      e.preventDefault();
      confirmRenameModal();
    }
  });
}

function folderNameFromPath(p) {
  if (!p) return '';
  const cleaned = p.replace(/\/+$/, '');
  const parts = cleaned.split('/');
  return parts[parts.length - 1] || p;
}

function buildProjectMenu(el) {
  const id = parseInt(el.dataset.id, 10);
  const name = el.dataset.name || '';
  const cwd = el.dataset.cwd || '';
  const isCurrent = id === _currentProjectId;
  return [
    {
      label: isCurrent ? '当前项目' : '切换到此项目',
      disabled: isCurrent,
      action: () => sendToBackend({ type: 'switch_project', id }),
    },
    {
      label: '在终端打开目录',
      action: () => {
        if (!isCurrent) sendToBackend({ type: 'switch_project', id });
        // 稍等项目切换后再 cd（当前项目则立即）
        setTimeout(() => cdToBookmark(cwd), isCurrent ? 0 : 120);
      },
    },
    { sep: true },
    {
      label: '复制路径',
      action: () => copyText(cwd),
    },
    {
      label: '重命名…',
      action: () => openRenameModal({
        title: '重命名项目',
        label: '项目名称',
        value: name,
        onConfirm: (newName) => {
          sendToBackend({ type: 'rename_project', id, name: newName });
        },
      }),
    },
    { sep: true },
    {
      label: '删除项目',
      danger: true,
      action: () => {
        if (confirm('确定删除项目「' + name + '」吗？该项目的历史和书签将一并删除。')) {
          sendToBackend({ type: 'delete_project', id });
        }
      },
    },
  ];
}

function buildRecentMenu(el) {
  const path = el.dataset.path || el.title || '';
  return [
    {
      label: '跳转到此目录',
      action: () => cdToBookmark(path),
    },
    {
      label: '复制路径',
      action: () => copyText(path),
    },
    { sep: true },
    {
      label: '添加为书签',
      action: () => {
        sendToBackend({
          type: 'add_bookmark',
          name: folderNameFromPath(path) || path,
          path,
        });
      },
    },
    {
      label: '新建项目…',
      action: () => openNewProjectModal({
        name: folderNameFromPath(path),
        cwd: path,
      }),
    },
  ];
}

function buildTabMenu(el) {
  const tabId = parseInt(el.dataset.tabId, 10);
  return [
    {
      label: '关闭标签',
      shortcut: 'Ctrl+W',
      action: () => closeTab(tabId),
    },
    {
      label: '关闭其他标签',
      action: () => {
        [...terminals.keys()].forEach((id) => {
          if (id !== tabId) closeTab(id);
        });
      },
    },
    { sep: true },
    {
      label: '新建标签',
      shortcut: 'Ctrl+T',
      action: () => newTab(),
    },
  ];
}

function buildNavMenu(el) {
  const view = el.dataset.view;
  return [
    {
      label: '打开',
      action: () => switchView(view),
    },
  ];
}

function buildSidebarBlankMenu() {
  return [
    {
      label: '新建项目…',
      action: () => openNewProjectModal(),
    },
    {
      label: '新建标签',
      shortcut: 'Ctrl+T',
      action: () => newTab(),
    },
  ];
}

function buildTerminalMenu() {
  const aid = getActiveTabId();
  const entry = aid !== null ? terminals.get(aid) : null;
  const term = entry ? entry.term : null;
  const hasSelection = !!(term && term.hasSelection && term.hasSelection());
  return [
    {
      label: '复制',
      shortcut: 'Ctrl+C',
      disabled: !hasSelection,
      action: () => {
        if (!term) return;
        const sel = term.getSelection();
        if (sel) {
          copyText(sel);
          try { term.clearSelection(); } catch (_) {}
        }
      },
    },
    {
      label: '粘贴',
      shortcut: 'Ctrl+V',
      action: () => {
        if (aid === null) return;
        pasteToTab(aid);
      },
    },
    {
      label: '中断',
      shortcut: 'Ctrl+Z',
      action: () => {
        if (aid === null) return;
        sendInterrupt(aid);
      },
    },
    { sep: true },
    {
      label: '清屏',
      action: () => {
        if (term) term.clear();
      },
    },
    {
      label: '新建标签',
      shortcut: 'Ctrl+T',
      action: () => newTab(),
    },
  ];
}

function resolveContextMenu(target) {
  if (!target || !target.closest) return null;

  const project = target.closest('.project-item');
  if (project) return buildProjectMenu(project);

  const recent = target.closest('#sidebar-recents .section-item:not(.empty-hint)');
  if (recent) return buildRecentMenu(recent);

  const tab = target.closest('.tab');
  if (tab) return buildTabMenu(tab);

  const nav = target.closest('.nav-item');
  if (nav) return buildNavMenu(nav);

  // 侧边栏空白区
  if (target.closest('#sidebar')) return buildSidebarBlankMenu();

  // 终端区
  if (target.closest('#terminals') || target.closest('.terminal-pane') || target.closest('.xterm')) {
    return buildTerminalMenu();
  }

  // 其他区域：不弹菜单，但仍阻止浏览器默认菜单
  return [];
}

function setupContextMenu() {
  setupRenameModal();

  // 全局拦截浏览器/WebKit 默认右键菜单
  document.addEventListener('contextmenu', (e) => {
    e.preventDefault();
    e.stopPropagation();

    // 输入框保留系统编辑菜单的体验：仍阻止 WebKit 默认（后退/检查元素），
    // 但对 input/textarea 提供基础编辑项
    const tag = (e.target && e.target.tagName) ? e.target.tagName.toLowerCase() : '';
    if (tag === 'input' || tag === 'textarea') {
      const el = e.target;
      showContextMenu(e.clientX, e.clientY, [
        {
          label: '剪切',
          disabled: el.selectionStart === el.selectionEnd,
          action: () => {
            const start = el.selectionStart;
            const end = el.selectionEnd;
            const selected = el.value.slice(start, end);
            if (!selected) return;
            copyText(selected);
            el.value = el.value.slice(0, start) + el.value.slice(end);
            el.setSelectionRange(start, start);
            el.dispatchEvent(new Event('input', { bubbles: true }));
          },
        },
        {
          label: '复制',
          disabled: el.selectionStart === el.selectionEnd,
          action: () => {
            copyText(el.value.slice(el.selectionStart, el.selectionEnd));
          },
        },
        {
          label: '粘贴',
          action: async () => {
            try {
              const text = await readClipboardText();
              if (!text) return;
              const start = el.selectionStart;
              const end = el.selectionEnd;
              el.value = el.value.slice(0, start) + text + el.value.slice(end);
              const pos = start + text.length;
              el.setSelectionRange(pos, pos);
              el.dispatchEvent(new Event('input', { bubbles: true }));
            } catch (err) {
              console.warn('[lotus] 输入框粘贴失败:', err);
            }
          },
        },
        {
          label: '全选',
          action: () => el.select(),
        },
      ]);
      return;
    }

    const items = resolveContextMenu(e.target);
    if (!items || items.length === 0) {
      hideContextMenu();
      return;
    }
    showContextMenu(e.clientX, e.clientY, items);
  }, true);

  // 点击别处 / 滚动 / 失焦 关闭菜单
  document.addEventListener('mousedown', (e) => {
    const menu = document.getElementById('ctx-menu');
    if (!menu || menu.classList.contains('hidden')) return;
    if (!menu.contains(e.target)) hideContextMenu();
  }, true);

  window.addEventListener('blur', hideContextMenu);
  window.addEventListener('resize', hideContextMenu);
  document.addEventListener('scroll', hideContextMenu, true);

  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') {
      const menu = document.getElementById('ctx-menu');
      if (menu && !menu.classList.contains('hidden')) {
        e.preventDefault();
        hideContextMenu();
        return;
      }
      const renameModal = document.getElementById('rename-modal');
      if (renameModal && !renameModal.classList.contains('hidden')) {
        e.preventDefault();
        closeRenameModal();
      }
    }
  });
}

if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', setupContextMenu);
} else {
  setupContextMenu();
}

// ====== Agent 工作流：启动器 + 徽章 + 完成通知 ======

function ensureTabRuntime(tabId) {
  if (!tabRuntime.has(tabId)) {
    tabRuntime.set(tabId, { busy: false, cmd: '', isAgent: false });
  }
  return tabRuntime.get(tabId);
}

function setTabBadge(tabId, state, title) {
  const badge = document.querySelector(`.tab[data-tab-id="${tabId}"] .tab-badge`);
  if (!badge) return;
  badge.classList.remove('busy', 'done', 'error');
  if (state) badge.classList.add(state);
  badge.title = title || '';
}

function clearTabDoneBadge(tabId) {
  const rt = tabRuntime.get(tabId);
  if (rt && rt.busy) return; // 忙碌中保留
  const badge = document.querySelector(`.tab[data-tab-id="${tabId}"] .tab-badge`);
  if (!badge) return;
  if (badge.classList.contains('done') || badge.classList.contains('error')) {
    badge.classList.remove('done', 'error');
    badge.title = '';
  }
}

function onCommandStarted(tabId, cmd) {
  const rt = ensureTabRuntime(tabId);
  rt.busy = true;
  rt.cmd = cmd || '';
  if (isAgentCommand(cmd)) {
    rt.isAgent = true;
    const tabEl = document.querySelector(`.tab[data-tab-id="${tabId}"]`);
    if (tabEl) tabEl.classList.add('has-agent');
    // 用 agent 名更新 tab 标题
    const short = String(cmd).trim().split(/\s+/)[0];
    updateTabTitle(tabId, short);
  }
  setTabBadge(tabId, 'busy', '运行中: ' + (cmd || ''));
}

function onCommandFinished(tabId, cmd, code, durationMs) {
  const rt = ensureTabRuntime(tabId);
  rt.busy = false;
  rt.cmd = '';
  const ok = code === 0;
  setTabBadge(
    tabId,
    ok ? 'done' : 'error',
    (ok ? '已完成' : '失败') + ': ' + (cmd || '') + (durationMs ? ` (${Math.round(durationMs / 100) / 10}s)` : '')
  );

  // 当前聚焦 tab 几秒后自动清徽章
  if (getActiveTabId() === tabId) {
    setTimeout(() => clearTabDoneBadge(tabId), 2500);
  }

  // agent 任务，或后台较长命令 → 桌面通知
  const isAgent = rt.isAgent || isAgentCommand(cmd);
  const notFocused = getActiveTabId() !== tabId;
  if (isAgent || (notFocused && durationMs >= 5000)) {
    const status = ok ? '完成' : '失败';
    const short = String(cmd || '').replace(/\n/g, ' ').slice(0, 60);
    sendToBackend({
      type: 'desktop_notify',
      title: isAgent ? `Agent ${status}` : `命令${status}`,
      body: short + (durationMs ? ` · ${(durationMs / 1000).toFixed(1)}s` : ''),
    });
  }
}

function renderAgentsList(agents) {
  const list = document.getElementById('agents-list');
  if (!list) return;
  if (!agents.length) {
    list.innerHTML = '<div class="empty-state">未探测到 Agent 目录</div>';
    return;
  }
  list.innerHTML = '';
  // 已安装的排前面
  const sorted = [...agents].sort((a, b) => Number(b.installed) - Number(a.installed));
  for (const a of sorted) {
    const card = document.createElement('div');
    card.className = 'agent-card' + (a.installed ? ' installed' : '');
    card.innerHTML =
      '<div class="agent-icon"></div>' +
      '<div class="agent-meta">' +
        '<div class="agent-name"></div>' +
        '<div class="agent-sub"></div>' +
      '</div>' +
      '<span class="agent-status"></span>' +
      '<button type="button" class="footer-btn primary agent-launch">启动</button>';
    card.querySelector('.agent-icon').textContent = a.icon || '✦';
    card.querySelector('.agent-name').textContent = a.name;
    card.querySelector('.agent-sub').textContent = a.installed
      ? (a.path || a.bin) + '  ·  `' + a.cmd + '`'
      : '未安装 `' + a.bin + '`（仍可尝试启动，若在 PATH 中）';
    const st = card.querySelector('.agent-status');
    st.textContent = a.installed ? '已安装' : '未检测到';
    st.className = 'agent-status ' + (a.installed ? 'ok' : 'miss');
    const btn = card.querySelector('.agent-launch');
    btn.addEventListener('click', () => launchAgent(a.cmd, a.name.split(' ')[0].toLowerCase() || a.id));
    list.appendChild(card);
  }
}

function launchAgent(command, title) {
  if (!command || !command.trim()) return;
  sendToBackend({
    type: 'launch_agent',
    command: command.trim(),
    title: title || command.trim().split(/\s+/)[0],
  });
  // 切回终端视图看启动过程
  switchView('terminal');
}

function setupAgentsPanel() {
  const refresh = document.getElementById('refresh-agents-btn');
  if (refresh) refresh.addEventListener('click', () => sendToBackend({ type: 'get_agents' }));
  const launchBtn = document.getElementById('custom-agent-launch');
  const cmdInput = document.getElementById('custom-agent-cmd');
  if (launchBtn && cmdInput) {
    launchBtn.addEventListener('click', () => {
      launchAgent(cmdInput.value, cmdInput.value.trim().split(/\s+/)[0]);
    });
    cmdInput.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') {
        e.preventDefault();
        launchBtn.click();
      }
    });
  }
  // 启动时预探测一次（不强制打开面板）
  setTimeout(() => sendToBackend({ type: 'get_agents' }), 400);
}

function setupAgentFeatures() {
  setupAgentsPanel();
}

if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', setupAgentFeatures);
} else {
  setupAgentFeatures();
}
