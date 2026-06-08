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
};

const $ = (id) => document.getElementById(id);

function showToast(message, type = "") {
  const el = $("toast");
  el.textContent = message;
  el.className = `toast ${type}`;
  setTimeout(() => el.classList.add("hidden"), 3200);
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
    li.className = state.selectedAccount?.fakeid === acc.fakeid ? "active" : "";
    li.innerHTML = `
      <div class="name">${escapeHtml(acc.nickname)}</div>
      <div class="meta">${escapeHtml(acc.fakeid)}</div>
    `;
    li.addEventListener("click", () => selectAccount(acc));
    list.appendChild(li);
  });
}

function renderArticles() {
  const grid = $("article-list");
  grid.innerHTML = "";
  state.selectedUrls.clear();
  updateDownloadButton();

  if (!state.selectedAccount) {
    grid.innerHTML = '<div class="empty">请先选择公众号</div>';
    return;
  }

  if (state.articles.length === 0) {
    grid.innerHTML = '<div class="empty">该页暂无文章</div>';
    return;
  }

  state.articles.forEach((article) => {
    const row = document.createElement("div");
    row.className = "article-item";
    const checked = state.selectedUrls.has(article.url);
    row.innerHTML = `
      <input type="checkbox" data-url="${escapeAttr(article.url)}" ${checked ? "checked" : ""} />
      <div>
        <div class="title">${escapeHtml(article.title)}</div>
        <div class="url">${escapeHtml(article.url)}</div>
      </div>
    `;
    const cb = row.querySelector("input");
    cb.addEventListener("change", () => {
      if (cb.checked) state.selectedUrls.add(article.url);
      else state.selectedUrls.delete(article.url);
      updateDownloadButton();
    });
    grid.appendChild(row);
  });
}

function updateDownloadButton() {
  const count = state.selectedUrls.size;
  const btn = $("btn-download");
  btn.textContent = `下载选中 (${count})`;
  btn.disabled = count === 0 || !state.outputDir;
}

function updatePager() {
  const page = Math.floor(state.pageBegin / PAGE_SIZE) + 1;
  $("page-info").textContent = `第 ${page} 页`;
  $("btn-prev").disabled = state.pageBegin === 0;
  $("btn-next").disabled = state.articles.length < PAGE_SIZE;
}

async function loadSettings() {
  try {
    const info = await invoke("get_settings");
    const lines = [
      `配置文件：${info.config_path}`,
      `密钥状态：${info.auth_key_set ? `已设置 (${info.auth_key_len} 字符)` : "未设置"}`,
    ];
    if (info.base_url) lines.push(`API 地址：${info.base_url}`);
    $("settings-status").textContent = lines.join("\n");
  } catch (err) {
    $("settings-status").textContent = `加载失败：${err}`;
  }
}

async function selectAccount(account) {
  state.selectedAccount = account;
  state.pageBegin = 0;
  $("article-pane-title").textContent = `${account.nickname} — 文章列表`;
  renderAccounts();
  await loadArticles();
}

async function loadArticles() {
  if (!state.selectedAccount) return;
  try {
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

async function downloadSelected() {
  const selected = state.articles.filter((a) => state.selectedUrls.has(a.url));
  if (selected.length === 0) return;

  const format = $("format").value;
  const interval = parseFloat($("interval").value) || 0;
  const wrap = $("progress-wrap");
  wrap.classList.remove("hidden");
  $("btn-download").disabled = true;

  try {
    const saved = await invoke("download_articles", {
      payload: {
        articles: selected,
        format,
        output_dir: state.outputDir,
        interval_secs: interval,
      },
    });
    showToast(`下载完成，共保存 ${saved} 篇`, "success");
  } catch (err) {
    showToast(String(err), "error");
  } finally {
    $("btn-download").disabled = false;
    setTimeout(() => wrap.classList.add("hidden"), 1500);
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

$("btn-save-auth").addEventListener("click", async () => {
  const authKey = $("auth-key").value.trim();
  try {
    const info = await invoke("save_auth_key", { payload: { auth_key: authKey } });
    showToast("密钥已保存", "success");
    const lines = [
      `配置文件：${info.config_path}`,
      `密钥状态：已设置 (${info.auth_key_len} 字符)`,
    ];
    $("settings-status").textContent = lines.join("\n");
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

listen("download-progress", (event) => {
  const p = event.payload;
  const pct = Math.round((p.current / p.total) * 100);
  $("progress-fill").style.width = `${pct}%`;
  $("progress-text").textContent =
    p.status === "done"
      ? `[${p.current}/${p.total}] 已保存：${p.title}`
      : `[${p.current}/${p.total}] 下载中：${p.title}`;
});

loadSettings();
