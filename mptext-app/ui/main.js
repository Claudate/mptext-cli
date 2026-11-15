const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const PAGE_SIZE = 10;

const state = {
  accounts: [],
  selectedAccount: null,
  articles: [],
  pageBegin: 0,
  selectedUrls: new Set(),
  // 见过的文章按 url 索引：分页浏览时只拿得到当前页，
  // 跨页勾选要下载就必须靠它把别页选中的文章找回来
  articleIndex: new Map(),
  outputDir: "",
  fetchedAll: false,
  busy: false,
};

const $ = (id) => document.getElementById(id);

function showToast(message, type = "") {
  const el = $("toast");
  el.textContent = message;
  el.className = `toast ${type}`;
  setTimeout(() => el.classList.add("hidden"), 3600);
}

/**
 * 进度面板：分阶段展示抓取/下载进度。
 * @param {object} opts
 * @param {string} opts.phase   阶段标题，如「正在抓取文章」「正在下载」
 * @param {string} opts.counter 右侧计数，如「128 篇」「12 / 128」
 * @param {number} opts.percent 进度条百分比 0-100；< 0 表示不确定（脉冲动画）
 * @param {string} opts.detail  详情 HTML，如当前标题、成功/失败明细
 * @param {boolean} opts.spinning 是否显示转圈
 * @param {string} opts.type    "" | success | warn | error
 */
function showProgress({
  phase = "",
  counter = "",
  percent = -1,
  detail = "",
  spinning = false,
  type = "",
} = {}) {
  const panel = $("progress-panel");
  panel.classList.remove("hidden");
  panel.className = `progress-panel${type ? " " + type : ""}`;
  $("progress-phase").textContent = phase;
  $("progress-counter").innerHTML = counter;
  $("progress-detail").innerHTML = detail;
  $("progress-spinner").style.display = spinning ? "block" : "none";

  const fill = $("progress-fill");
  const track = fill.parentElement;
  if (percent < 0) {
    track.classList.add("indeterminate");
    fill.style.width = "100%";
  } else {
    track.classList.remove("indeterminate");
    fill.style.width = `${Math.min(100, Math.max(0, percent))}%`;
  }
}

function hideProgress(delay = 0) {
  if (delay > 0) {
    setTimeout(() => $("progress-panel").classList.add("hidden"), delay);
  } else {
    $("progress-panel").classList.add("hidden");
  }
}

function switchTab(name) {
  document.querySelectorAll(".tab").forEach((btn) => {
    btn.classList.toggle("active", btn.dataset.tab === name);
  });
  document.querySelectorAll(".panel").forEach((panel) => {
    panel.classList.toggle("active", panel.id === `panel-${name}`);
  });
}

function renderAccounts() {
  const list = $("account-list");
  list.innerHTML = "";
  $("account-count").textContent = String(state.accounts.length);

  if (state.accounts.length === 0) {
    list.innerHTML = '<li class="empty">搜索公众号后在此显示</li>';
    return;
  }

  state.accounts.forEach((acc) => {
    const li = document.createElement("li");
    const isActive = state.selectedAccount?.fakeid === acc.fakeid;
    li.className = `account-row${isActive ? " active" : ""}${acc.favorite ? " fav" : ""}`;
    const starFill = acc.favorite ? "currentColor" : "none";
    li.innerHTML = `
      <div class="acc-main">
        <div class="name">${escapeHtml(acc.nickname)}</div>
        <div class="meta">${escapeHtml(acc.alias || acc.fakeid)}</div>
      </div>
      <div class="acc-actions">
        <button class="mini-btn star-btn${acc.favorite ? " on" : ""}" title="${acc.favorite ? "取消收藏" : "收藏"}">
          <svg viewBox="0 0 24 24" width="16" height="16"><path d="M12 3.5l2.6 5.3 5.9.9-4.3 4.1 1 5.8-5.2-2.7-5.2 2.7 1-5.8L3.5 9.7l5.9-.9z" fill="${starFill}" stroke="currentColor" stroke-width="1.4" stroke-linejoin="round"/></svg>
        </button>
        <button class="mini-btn del-btn" title="删除">
          <svg viewBox="0 0 24 24" width="15" height="15"><path d="M5 7h14M9 7V5h6v2M7 7l1 12h8l1-12" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"/></svg>
        </button>
      </div>
    `;
    li.querySelector(".acc-main").addEventListener("click", () => selectAccount(acc));
    li.querySelector(".star-btn").addEventListener("click", (e) => {
      e.stopPropagation();
      toggleFavorite(acc.fakeid);
    });
    li.querySelector(".del-btn").addEventListener("click", (e) => {
      e.stopPropagation();
      removeAccount(acc.fakeid, acc.nickname);
    });
    list.appendChild(li);
  });
}

async function toggleFavorite(fakeid) {
  try {
    const updated = await invoke("toggle_favorite", { fakeid });
    state.accounts = updated;
    renderAccounts();
  } catch (err) {
    showToast(String(err), "error");
  }
}

async function removeAccount(fakeid, nickname) {
  try {
    const updated = await invoke("remove_account", { fakeid });
    state.accounts = updated;
    if (state.selectedAccount?.fakeid === fakeid) {
      state.selectedAccount = null;
      state.articles = [];
      state.selectedUrls.clear();
      renderArticles();
      updatePager();
    }
    renderAccounts();
    showToast(`已删除「${nickname}」`, "success");
  } catch (err) {
    showToast(String(err), "error");
  }
}

function renderArticles() {
  const grid = $("article-list");
  grid.innerHTML = "";
  updateDownloadButton();

  if (!state.selectedAccount) {
    grid.innerHTML = '<div class="empty">请先选择公众号</div>';
    return;
  }

  if (state.articles.length === 0) {
    grid.innerHTML = state.fetchedAll
      ? '<div class="empty">该公众号暂无可下载的文章</div>'
      : '<div class="empty">该页暂无文章</div>';
    return;
  }

  state.articles.forEach((article, idx) => {
    const row = document.createElement("div");
    // 分页浏览时序号接全局偏移，别每页都从 #1 重来
    const seq = (state.fetchedAll ? 0 : state.pageBegin) + idx + 1;
    row.className = "article-item";
    row.dataset.rowUrl = article.url;
    const checked = state.selectedUrls.has(article.url);
    // 每页渲染时都把文章存进索引，跨页下载才有完整对象
    state.articleIndex.set(article.url, article);
    const time = formatArticleTime(article.create_time);
    const metaParts = [`#${seq}`];
    if (time) metaParts.push(time);
    row.innerHTML = `
      <input type="checkbox" class="article-check" data-url="${escapeAttr(article.url)}" ${checked ? "checked" : ""} />
      <div class="article-body">
        <div class="title">${escapeHtml(article.title)}</div>
        <div class="art-meta">
          <span class="art-index">${metaParts.join(" · ")}</span>
          <span class="url">${escapeHtml(article.url)}</span>
        </div>
      </div>
      <span class="dl-badge" data-badge="idle"></span>
    `;
    const cb = row.querySelector("input");
    cb.addEventListener("change", () => {
      if (cb.checked) state.selectedUrls.add(article.url);
      else state.selectedUrls.delete(article.url);
      updateDownloadButton();
      syncCheckAll();
    });
    grid.appendChild(row);
  });
  syncCheckAll();
}

/** 把文章时间戳（Unix 秒，可能是字符串/数字）格式化为 YYYY-MM-DD */
function formatArticleTime(raw) {
  if (raw === null || raw === undefined) return "";
  const n = typeof raw === "string" ? parseInt(raw, 10) : Number(raw);
  if (!Number.isFinite(n) || n <= 0) return "";
  const d = new Date(n * 1000);
  if (Number.isNaN(d.getTime())) return "";
  const pad = (x) => String(x).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

function setArticleBadge(url, status) {
  const row = document.querySelector(`.article-item[data-row-url="${cssEscape(url)}"]`);
  if (!row) return;
  const badge = row.querySelector(".dl-badge");
  if (!badge) return;
  badge.dataset.badge = status;
  const labels = {
    idle: "",
    downloading: "下载中",
    done: "已下载",
    failed: "失败",
  };
  badge.textContent = labels[status] ?? "";
  if (status === "downloading" || status === "done") {
    row.scrollIntoView({ block: "nearest", behavior: "smooth" });
  }
}

function cssEscape(s) {
  return String(s).replace(/["\\]/g, "\\$&");
}

function resetArticleBadges() {
  document.querySelectorAll(".dl-badge").forEach((b) => {
    b.dataset.badge = "idle";
    b.textContent = "";
  });
}

function syncCheckAll() {
  const checkAll = $("check-all");
  if (!checkAll) return;
  const total = state.articles.length;
  if (total === 0) {
    checkAll.checked = false;
    checkAll.indeterminate = false;
    checkAll.disabled = true;
    return;
  }
  checkAll.disabled = false;
  const selectedOnPage = state.articles.filter((a) =>
    state.selectedUrls.has(a.url),
  ).length;
  checkAll.checked = selectedOnPage === total;
  checkAll.indeterminate = selectedOnPage > 0 && selectedOnPage < total;
}

function toggleSelectAll(checked) {
  state.articles.forEach((a) => {
    if (checked) state.selectedUrls.add(a.url);
    else state.selectedUrls.delete(a.url);
  });
  document.querySelectorAll(".article-check").forEach((cb) => {
    cb.checked = checked;
  });
  updateDownloadButton();
  syncCheckAll();
}

function updateDownloadButton() {
  const busy = state.busy;
  const count = state.selectedUrls.size;

  const btn = $("btn-download");
  btn.textContent = `下载选中 (${count})`;
  btn.disabled = busy || count === 0 || !state.outputDir;

  // 一键下载全部：选好公众号 + 目录即可，无需先勾选
  const autoBtn = $("btn-auto");
  if (autoBtn) {
    autoBtn.disabled = busy || !state.selectedAccount || !state.outputDir;
    autoBtn.title = !state.selectedAccount
      ? "请先在左侧选择一个公众号"
      : !state.outputDir
        ? "请先选择保存目录"
        : "自动抓取该公众号全部文章并立即下载，中途无需操作";
  }

  const fetchAllBtn = $("btn-fetch-all");
  if (fetchAllBtn) {
    fetchAllBtn.disabled = busy || !state.selectedAccount;
  }
}

/** 进入/退出"忙碌"态：统一锁定操作按钮，避免重复触发 */
function setBusy(busy) {
  state.busy = busy;
  ["btn-search", "btn-pick-dir", "btn-prev", "btn-next"].forEach((id) => {
    const el = $(id);
    if (el) el.disabled = busy || el.dataset.lockExempt === "1";
  });
  updateDownloadButton();
}

function updatePager() {
  if (state.fetchedAll) {
    $("page-info").textContent = `全部 ${state.articles.length} 篇`;
    $("btn-prev").disabled = true;
    $("btn-next").disabled = true;
    return;
  }
  const page = Math.floor(state.pageBegin / PAGE_SIZE) + 1;
  $("page-info").textContent = `第 ${page} 页`;
  $("btn-prev").disabled = state.pageBegin === 0;
  $("btn-next").disabled = state.articles.length < PAGE_SIZE;
}

function renderSettingsStatus(info) {
  const box = $("settings-status");
  const who = info.nickname ? `：${escapeHtml(info.nickname)}` : "";
  const memo = info.logged_in
    ? `<span class="ok-dot"></span>已登录${who}，下次打开自动使用`
    : `<span class="warn-dot"></span>尚未登录`;
  const lines = [`<div class="memo-line">${memo}</div>`];
  lines.push(`<div class="path-line">会话文件：${escapeHtml(info.session_path || "")}</div>`);
  box.innerHTML = lines.join("");
}

async function loadSettings() {
  try {
    const info = await invoke("get_settings");
    renderSettingsStatus(info);
  } catch (err) {
    $("settings-status").textContent = `加载失败：${err}`;
  }
}

async function loadSavedAccounts() {
  try {
    const saved = await invoke("get_saved_accounts");
    if (Array.isArray(saved) && saved.length > 0) {
      state.accounts = saved;
      renderAccounts();
    }
  } catch (err) {
    console.error("加载公众号记忆失败:", err);
  }
}

async function selectAccount(account) {
  state.selectedAccount = account;
  state.pageBegin = 0;
  state.fetchedAll = false;
  state.selectedUrls.clear();
  state.articleIndex.clear();
  $("article-pane-title").textContent = `${account.nickname} — 文章列表`;
  renderAccounts();
  updateDownloadButton();
  await loadArticles();
}

/// 加载当前页。返回 true 表示成功；调用方据此决定失败时要不要回滚偏移。
async function loadArticles() {
  if (!state.selectedAccount) return false;
  try {
    state.fetchedAll = false;
    // 不清空 selectedUrls：勾选要能跨页保留，否则翻一页就丢掉之前选的
    state.articles = await invoke("list_articles", {
      payload: {
        fakeid: state.selectedAccount.fakeid,
        begin: state.pageBegin,
        size: PAGE_SIZE,
      },
    });
    renderArticles();
    updatePager();
    return true;
  } catch (err) {
    // 失败必须看得见：否则界面停在上页，用户会以为「下一页」按钮没反应
    $("page-info").textContent = "加载失败";
    $("page-info").title = String(err);
    const grid = $("article-list");
    grid.innerHTML = "";
    const box = document.createElement("div");
    box.className = "empty";
    box.textContent = `加载失败：${String(err)}`;
    grid.appendChild(box);
    showToast(String(err), "error");
    return false;
  }
}

async function searchAccounts() {
  const keyword = $("keyword").value.trim();
  if (!keyword) {
    showToast("请输入搜索关键词", "error");
    return;
  }
  $("btn-search").disabled = true;
  try {
    state.accounts = await invoke("search_accounts", { keyword });
    state.selectedAccount = null;
    state.articles = [];
    state.pageBegin = 0;
    state.fetchedAll = false;
    state.selectedUrls.clear();
    state.articleIndex.clear();
    renderAccounts();
    renderArticles();
    updatePager();
    if (state.accounts.length === 0) showToast("未找到匹配的公众号");
    else showToast(`找到 ${state.accounts.length} 个公众号`, "success");
  } catch (err) {
    showToast(String(err), "error");
  } finally {
    $("btn-search").disabled = false;
  }
}

async function pickOutputDir() {
  const dir = await invoke("pick_output_dir");
  if (dir) {
    state.outputDir = dir;
    $("output-dir").textContent = dir;
    updateDownloadButton();
  }
}

// 下载计数器：done=成功篇数，failed=失败篇数，total=本轮总数
const dlCounter = { done: 0, failed: 0, total: 0 };

function resetDlCounter(total = 0) {
  dlCounter.done = 0;
  dlCounter.failed = 0;
  dlCounter.total = total;
}

/** 统一渲染"下载进度"面板（被一键下载与下载选中共用） */
function renderDownloadProgress(currentTitle = "") {
  const { done, failed, total } = dlCounter;
  const finished = done + failed;
  const percent = total > 0 ? (finished / total) * 100 : 0;
  const detail = currentTitle
    ? `<span class="cur-title">正在下载：${escapeHtml(currentTitle)}</span>`
    : "";
  showProgress({
    phase: "正在下载文章",
    counter:
      `<b>${finished}</b> / ${total} 篇` +
      ` · <span class="ok-text">成功 ${done}</span>` +
      (failed > 0 ? ` · <span class="err-text">失败 ${failed}</span>` : ""),
    percent,
    detail,
    spinning: true,
  });
}

/** 下载结束后的汇总展示（成功/含失败） */
function finishDownloadProgress(saved, failedCount) {
  if (failedCount === 0) {
    showProgress({
      phase: "全部下载完成",
      counter: `共 <b>${saved}</b> 篇 · 全部成功`,
      percent: 100,
      type: "success",
    });
    hideProgress(5000);
    showToast(`下载完成，共保存 ${saved} 篇`, "success");
  } else {
    showProgress({
      phase: "下载完成（含失败）",
      counter: `<span class="ok-text">成功 ${saved}</span> · <span class="err-text">失败 ${failedCount}</span>`,
      percent: 100,
      detail: `失败的文章已在列表中标红，可再次点击「下载选中」重试`,
      type: "warn",
    });
    hideProgress(7000);
    showToast(`完成：成功 ${saved} 篇，失败 ${failedCount} 篇（失败项已标红，可重试）`, "error");
  }
}

async function downloadSelected() {
  // 按 url 从全量索引取，而不是只筛当前页的 state.articles
  const selected = [...state.selectedUrls]
    .map((url) => state.articleIndex.get(url))
    .filter(Boolean);
  if (selected.length === 0) return;
  if (!state.outputDir) {
    showToast("请先选择保存目录", "error");
    return;
  }

  const format = $("format").value;
  const interval = parseFloat($("interval").value) || 0;

  resetDlCounter(selected.length);
  resetArticleBadges();
  setBusy(true);
  showProgress({ phase: "准备下载…", counter: `共 <b>${selected.length}</b> 篇`, percent: 0, spinning: true });

  try {
    const result = await invoke("download_articles", {
      payload: {
        articles: selected,
        format,
        output_dir: state.outputDir,
        interval_secs: interval,
      },
    });
    finishDownloadProgress(result?.saved ?? 0, (result?.failed ?? []).length);
  } catch (err) {
    showProgress({ phase: "下载出错", detail: escapeHtml(String(err)), type: "error" });
    hideProgress(5000);
    showToast(String(err), "error");
  } finally {
    setBusy(false);
  }
}

/**
 * 一键下载全部（用户首选流程）：
 * 选好公众号 + 目录 → 点一下 → 自动抓取该号全部文章 → 立即逐篇下载，中途无需操作。
 * 抓取阶段进度走 fetch-articles-progress；列表就绪后由 auto-articles-ready 渲染并全选；
 * 下载阶段进度走 download-progress。
 */
async function autoDownloadAll() {
  if (!state.selectedAccount) {
    showToast("请先在左侧选择一个公众号", "error");
    return;
  }
  if (!state.outputDir) {
    showToast("请先选择保存目录", "error");
    return;
  }

  const format = $("format").value;
  const interval = parseFloat($("interval").value) || 0;

  setBusy(true);
  // 进入抓取阶段：先重置列表与计数，进度条走不确定脉冲
  state.fetchedAll = false;
  state.articles = [];
  state.selectedUrls.clear();
  resetDlCounter(0);
  renderArticles();
  showProgress({
    phase: "正在抓取文章",
    counter: "已获取 <b>0</b> 篇",
    percent: -1,
    spinning: true,
  });

  try {
    const result = await invoke("auto_download_all", {
      payload: {
        fakeid: state.selectedAccount.fakeid,
        format,
        output_dir: state.outputDir,
        interval_secs: interval,
      },
    });
    const fetched = result?.fetched ?? 0;
    const saved = result?.saved ?? 0;
    const failed = result?.failed ?? [];
    if (fetched === 0) {
      showProgress({ phase: "该公众号暂无可下载的文章", percent: 0, type: "warn" });
      hideProgress(3500);
      showToast("未抓取到可下载的文章", "error");
    } else {
      finishDownloadProgress(saved, failed.length);
    }
  } catch (err) {
    showProgress({ phase: "执行失败", detail: escapeHtml(String(err)), type: "error" });
    hideProgress(5000);
    showToast(String(err), "error");
  } finally {
    setBusy(false);
  }
}

async function fetchAllArticles() {
  if (!state.selectedAccount) {
    showToast("请先选择公众号", "error");
    return;
  }
  setBusy(true);
  showProgress({ phase: "正在抓取文章", counter: "已获取 <b>0</b> 篇", percent: -1, spinning: true });
  $("article-list").innerHTML = '<div class="empty">正在抓取文章，请稍候…</div>';

  try {
    const articles = await invoke("fetch_all_articles", {
      fakeid: state.selectedAccount.fakeid,
    });
    state.articles = Array.isArray(articles) ? articles : [];
    state.fetchedAll = true;
    state.articleIndex.clear();
    state.articles.forEach((a) => state.articleIndex.set(a.url, a));
    // 抓取成功的全部默认选中（后端已过滤无效条目）
    state.selectedUrls = new Set(state.articles.map((a) => a.url));
    renderArticles();
    updatePager();
    if (state.articles.length === 0) {
      showProgress({ phase: "该公众号暂无可下载的文章", percent: 0, type: "warn" });
      hideProgress(2800);
    } else {
      showProgress({
        phase: "抓取完成",
        counter: `共 <b>${state.articles.length}</b> 篇 · 已默认全选`,
        percent: 100,
        type: "success",
      });
      hideProgress(3500);
      showToast(`已抓取全部 ${state.articles.length} 篇，已默认全选`, "success");
    }
  } catch (err) {
    showProgress({ phase: "抓取失败", detail: escapeHtml(String(err)), type: "error" });
    hideProgress(4000);
  } finally {
    setBusy(false);
  }
}

function escapeHtml(s) {
  return String(s)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function escapeAttr(s) {
  return escapeHtml(s).replaceAll("'", "&#39;");
}

document.querySelectorAll(".tab").forEach((btn) => {
  btn.addEventListener("click", () => switchTab(btn.dataset.tab));
});

$("btn-search").addEventListener("click", searchAccounts);
$("keyword").addEventListener("keydown", (e) => {
  if (e.key === "Enter") searchAccounts();
});

/**
 * 翻页：先挪偏移再取，取失败就把偏移退回去。
 * 不回滚的话一次失败会让 pageBegin 永久跑偏，之后每点一次都多跳过一页。
 */
async function turnPage(delta) {
  if (!state.selectedAccount || state.busy) return;
  const previous = state.pageBegin;
  state.pageBegin = Math.max(0, previous + delta * PAGE_SIZE);
  const ok = await loadArticles();
  if (!ok) {
    state.pageBegin = previous;
    updatePager();
  }
}

$("btn-prev").addEventListener("click", async () => {
  await turnPage(-1);
});

$("btn-next").addEventListener("click", async () => {
  await turnPage(1);
});

$("btn-pick-dir").addEventListener("click", pickOutputDir);
$("btn-download").addEventListener("click", downloadSelected);
$("btn-auto").addEventListener("click", autoDownloadAll);
$("btn-fetch-all").addEventListener("click", fetchAllArticles);
$("check-all").addEventListener("change", (e) => toggleSelectAll(e.target.checked));

$("btn-login").addEventListener("click", async () => {
  const btn = $("btn-login");
  btn.disabled = true;
  $("login-status").textContent = "正在向微信申请二维码…";
  try {
    const info = await invoke("start_login");
    renderSettingsStatus(info);
    const who = info.nickname ? `：${info.nickname}` : "";
    $("login-status").textContent = `登录成功${who}`;
    showToast("登录成功，会话已保存在本机", "success");
  } catch (err) {
    $("login-status").textContent = String(err);
    showToast(String(err), "error");
  } finally {
    btn.disabled = false;
  }
});

$("btn-clear-auth").addEventListener("click", async () => {
  try {
    const info = await invoke("logout");
    renderSettingsStatus(info);
    $("login-qr").hidden = true;
    $("login-qr").removeAttribute("src");
    $("login-status").textContent = "已退出。点「扫码登录」重新获取二维码";
    showToast("已退出登录", "success");
  } catch (err) {
    showToast(String(err), "error");
  }
});

$("btn-clear-accounts").addEventListener("click", async () => {
  try {
    await invoke("clear_saved_accounts");
    state.accounts = [];
    state.selectedAccount = null;
    state.articles = [];
    state.selectedUrls.clear();
    renderAccounts();
    renderArticles();
    updatePager();
    showToast("已清空记忆的公众号", "success");
  } catch (err) {
    showToast(String(err), "error");
  }
});

$("btn-verify-auth").addEventListener("click", async () => {
  try {
    const ok = await invoke("verify_session");
    showToast(ok ? "登录仍然有效" : "登录已失效，请重新扫码", ok ? "success" : "error");
    if (!ok) {
      $("login-status").textContent = "登录已失效，请重新扫码";
    }
    await loadSettings();
  } catch (err) {
    showToast(String(err), "error");
  }
});

listen("login-qr", (event) => {
  const img = $("login-qr");
  img.src = event.payload.image;
  img.hidden = false;
});

listen("login-status", (event) => {
  if (typeof event.payload === "string" && event.payload) {
    $("login-status").textContent = event.payload;
  }
});

// 抓取阶段进度：实时显示已获取篇数与当前页
listen("fetch-articles-progress", (event) => {
  const p = event.payload;
  if (p.status === "done") {
    showProgress({
      phase: "抓取完成，准备下载",
      counter: `共 <b>${p.fetched}</b> 篇`,
      percent: -1,
      spinning: true,
    });
  } else {
    // page 现在是通道序号（1 主页栏目 / 2 搜狗索引 / 3 登录态），
    // 显示通道名比显示「第 N 页」更接近真实进度
    showProgress({
      phase: "正在抓取文章",
      counter: `已获取 <b>${p.fetched}</b> 篇`,
      percent: -1,
      detail: `<span class="cur-title">${escapeHtml(p.status)}…</span>`,
      spinning: true,
    });
  }
});

// 一键流程：抓取列表就绪后立即渲染并默认全选（下载随后自动开始）
listen("auto-articles-ready", (event) => {
  const articles = Array.isArray(event.payload) ? event.payload : [];
  state.articles = articles;
  state.fetchedAll = true;
  state.articleIndex.clear();
  articles.forEach((a) => state.articleIndex.set(a.url, a));
  state.selectedUrls = new Set(articles.map((a) => a.url));
  resetDlCounter(articles.length);
  renderArticles();
  updatePager();
});

// 下载阶段进度：事件已带 url，精确定位行徽章
listen("download-progress", (event) => {
  const p = event.payload;
  const url = p.url || state.articles.find((a) => a.title === p.title)?.url;

  if (p.status === "downloading") {
    if (url) setArticleBadge(url, "downloading");
  } else if (p.status === "done") {
    dlCounter.done += 1;
    if (url) setArticleBadge(url, "done");
  } else if (p.status === "failed") {
    dlCounter.failed += 1;
    if (url) setArticleBadge(url, "failed");
  }

  // total 以后端为准（一键流程下计数器在 ready 事件里已设好）
  if (p.total) dlCounter.total = p.total;
  renderDownloadProgress(p.title);
});

// ---------------------------------------------------------------------------
// 全量历史：微信客户端抓包 + 作者通道
// ---------------------------------------------------------------------------

let mitmTimer = null;

function renderMitmState(s) {
  $("mitm-port-label").textContent = String(s.port);
  $("ca-path").textContent = s.ca_path || "-";
  const lines = [
    `代理：<b>${s.running ? `运行中（127.0.0.1:${s.port}）` : "未运行"}</b>`,
    `证书：<b>${s.ca_trusted ? "已被系统信任" : "尚未信任（首次需点「装信任证书」）"}</b>`,
    `已抓公众号：<b>${s.captured_accounts}</b>`,
  ];
  // 后端 message 只在代理运行时有增量信息，未运行时会和第一行重复
  if (s.running && s.message) lines.push(escapeHtml(s.message));
  $("mitm-status").innerHTML = lines.join("<br>");
  $("btn-mitm-start").disabled = s.running;
  $("btn-mitm-stop").disabled = !s.running;
}

async function refreshMitm() {
  try {
    const s = await invoke("mitm_status");
    renderMitmState(s);
  } catch (err) {
    $("mitm-status").textContent = `读取状态失败：${err}`;
  }
  try {
    const list = await invoke("list_captured_accounts");
    const ul = $("captured-list");
    if (!Array.isArray(list) || list.length === 0) {
      ul.innerHTML = '<li class="empty">还没抓到。开启代理后在微信里打开一篇文章。</li>';
    } else {
      ul.innerHTML = list
        .map(
          (a) => `
        <li class="captured-row">
          <div class="cap-main">
            <div class="cap-biz">${escapeHtml(a.biz || "（未知号）")}</div>
            <div class="cap-meta">
              票据 ${a.has_ticket ? "完整" : "不完整"} ·
              Cookie ${a.cookie_valid ? "有效" : "无/过期"}
              ${a.author_id ? ` · author_id ${escapeHtml(a.author_id)}` : ""}
            </div>
          </div>
        </li>`,
        )
        .join("");
    }
  } catch (err) {
    $("captured-list").innerHTML = `<li class="empty">${escapeHtml(String(err))}</li>`;
  }
}

/** 抓包期间轮询状态，让「已抓到 N 个公众号」实时变化。 */
function startMitmPolling() {
  if (mitmTimer) return;
  mitmTimer = setInterval(refreshMitm, 2000);
}

function stopMitmPolling() {
  if (mitmTimer) {
    clearInterval(mitmTimer);
    mitmTimer = null;
  }
}

$("btn-mitm-trust").addEventListener("click", async () => {
  try {
    showToast("系统会弹出授权框，请点「允许」…");
    const s = await invoke("mitm_trust_ca");
    renderMitmState(s);
    showToast("证书已信任，可以开启代理了", "success");
  } catch (err) {
    showToast(String(err), "error");
  }
});

$("btn-mitm-start").addEventListener("click", async () => {
  try {
    const s = await invoke("mitm_start", { port: 8899 });
    renderMitmState(s);
    startMitmPolling();
    showToast("代理已启动，接着点「指向系统代理」", "success");
  } catch (err) {
    showToast(String(err), "error");
  }
});

$("btn-mitm-stop").addEventListener("click", async () => {
  const s = await invoke("mitm_stop");
  renderMitmState(s);
  stopMitmPolling();
  showToast("代理已停止");
});

$("btn-mitm-proxy-on").addEventListener("click", async () => {
  try {
    const s = await invoke("mitm_set_proxy", { enabled: true, port: mitmPort() });
    renderMitmState(s);
    showToast("系统代理已指向本地抓包端口", "success");
  } catch (err) {
    showToast(String(err), "error");
  }
});

$("btn-mitm-proxy-off").addEventListener("click", async () => {
  try {
    const s = await invoke("mitm_set_proxy", { enabled: false, port: mitmPort() });
    renderMitmState(s);
    showToast("系统代理已恢复", "success");
  } catch (err) {
    showToast(String(err), "error");
  }
});

function mitmPort() {
  const n = parseInt($("mitm-port-label").textContent, 10);
  return Number.isFinite(n) ? n : 8899;
}

$("btn-clear-captures").addEventListener("click", async () => {
  try {
    await invoke("clear_captured_accounts");
    await refreshMitm();
    showToast("已清空抓到的票据", "success");
  } catch (err) {
    showToast(String(err), "error");
  }
});

$("btn-author-fetch").addEventListener("click", async () => {
  const url = $("author-url").value.trim();
  if (!url) {
    showToast("请先贴一篇该公众号的文章链接", "error");
    return;
  }
  const btn = $("btn-author-fetch");
  btn.disabled = true;
  $("author-result").textContent = "正在反查公众号并抓取，首次可能要几十秒…";
  $("author-preview").innerHTML = "";
  try {
    const result = await invoke("list_articles_by_author", {
      payload: { url, max_pages: 0 },
    });
    const articles = Array.isArray(result.articles) ? result.articles : [];
    const channels = Array.isArray(result.channels) ? result.channels : [];
    $("author-result").innerHTML = [
      `公众号：<b>${escapeHtml(result.nickname || "（未能识别昵称）")}</b>`,
      `__biz：<code>${escapeHtml(result.biz || "-")}</code>`,
      `共 <b>${articles.length}</b> 篇`,
      `通道：${channels.map(escapeHtml).join(" + ") || "-"}`,
      result.ticket_used
        ? "已叠加微信客户端票据的全量历史"
        : "未使用客户端票据（免登录通道已生效）",
    ].join("<br>");

    if (articles.length > 0) {
      // 复用搜索下载页的列表与下载流程：切过去、全选、用户点「下载选中」。
      state.selectedAccount = { fakeid: "", nickname: "作者全量历史" };
      state.articles = articles;
      state.fetchedAll = true;
      state.articleIndex.clear();
      articles.forEach((a) => state.articleIndex.set(a.url, a));
      state.selectedUrls = new Set(articles.map((a) => a.url));
      renderArticles();
      updatePager();
      $("article-pane-title").textContent = "作者全量历史";
      $("author-preview").innerHTML = articles
        .slice(0, 8)
        .map(
          (a) =>
            `<div class="prev-row">${escapeHtml(a.title)}</div>`,
        )
        .join("");
      showToast(`拉到 ${articles.length} 篇，已切到「搜索下载」页并全选`, "success");
    } else {
      $("author-preview").innerHTML = '<div class="empty">没有拉到文章</div>';
    }
  } catch (err) {
    $("author-result").innerHTML = `<span class="err">${escapeHtml(String(err))}</span>`;
    showToast(String(err), "error");
  } finally {
    btn.disabled = false;
  }
});

loadSettings();
loadSavedAccounts();
refreshMitm();
