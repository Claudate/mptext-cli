const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const PAGE_SIZE = 20;

const state = {
  accounts: [],
  selectedAccount: null,
  articles: [],
  pageBegin: 0,
  selectedUrls: new Set(),
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
    grid.innerHTML = '<div class="empty">该页暂无文章</div>';
    return;
  }

  state.articles.forEach((article, idx) => {
    const row = document.createElement("div");
    row.className = "article-item";
    row.dataset.rowUrl = article.url;
    const checked = state.selectedUrls.has(article.url);
    const time = formatArticleTime(article.create_time);
    const metaParts = [`#${idx + 1}`];
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
  const memo = info.auth_key_set
    ? `<span class="ok-dot"></span>Token 已记忆（${info.auth_key_len} 字符），下次打开自动加载`
    : `<span class="warn-dot"></span>尚未保存 Token`;
  const lines = [`<div class="memo-line">${memo}</div>`];
  lines.push(`<div class="path-line">配置文件：${escapeHtml(info.config_path)}</div>`);
  if (info.base_url) lines.push(`<div class="path-line">API 地址：${escapeHtml(info.base_url)}</div>`);
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
  $("article-pane-title").textContent = `${account.nickname} — 文章列表`;
  renderAccounts();
  updateDownloadButton();
  await loadArticles();
}

async function loadArticles() {
  if (!state.selectedAccount) return;
  try {
    state.fetchedAll = false;
    state.selectedUrls.clear();
    state.articles = await invoke("list_articles", {
      payload: {
        fakeid: state.selectedAccount.fakeid,
        begin: state.pageBegin,
        size: PAGE_SIZE,
      },
    });
    renderArticles();
    updatePager();
  } catch (err) {
    showToast(String(err), "error");
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
  const selected = state.articles.filter((a) => state.selectedUrls.has(a.url));
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

$("btn-prev").addEventListener("click", async () => {
  state.pageBegin = Math.max(0, state.pageBegin - PAGE_SIZE);
  await loadArticles();
});

$("btn-next").addEventListener("click", async () => {
  state.pageBegin += PAGE_SIZE;
  await loadArticles();
});

$("btn-pick-dir").addEventListener("click", pickOutputDir);
$("btn-download").addEventListener("click", downloadSelected);
$("btn-auto").addEventListener("click", autoDownloadAll);
$("btn-fetch-all").addEventListener("click", fetchAllArticles);
$("check-all").addEventListener("change", (e) => toggleSelectAll(e.target.checked));

$("btn-save-auth").addEventListener("click", async () => {
  const authKey = $("auth-key").value.trim();
  try {
    const info = await invoke("save_auth_key", { payload: { auth_key: authKey } });
    showToast("密钥已保存，已永久记忆", "success");
    renderSettingsStatus(info);
    $("auth-key").value = "";
  } catch (err) {
    showToast(String(err), "error");
  }
});

$("btn-clear-auth").addEventListener("click", async () => {
  try {
    const info = await invoke("clear_auth_key");
    showToast("已清除记忆的 Token", "success");
    renderSettingsStatus(info);
    $("auth-key").value = "";
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
    const ok = await invoke("verify_auth");
    showToast(ok ? "API 密钥有效" : "API 密钥已过期，请重新获取", ok ? "success" : "error");
  } catch (err) {
    showToast(String(err), "error");
  }
});

$("link-site").addEventListener("click", (e) => {
  e.preventDefault();
  showToast("请在浏览器打开 down.mptext.top 获取密钥");
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
    showProgress({
      phase: "正在抓取文章",
      counter: `已获取 <b>${p.fetched}</b> 篇`,
      percent: -1,
      detail: `<span class="cur-title">正在读取第 ${p.page} 页…</span>`,
      spinning: true,
    });
  }
});

// 一键流程：抓取列表就绪后立即渲染并默认全选（下载随后自动开始）
listen("auto-articles-ready", (event) => {
  const articles = Array.isArray(event.payload) ? event.payload : [];
  state.articles = articles;
  state.fetchedAll = true;
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

loadSettings();
loadSavedAccounts();
