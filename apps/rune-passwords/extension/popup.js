// The toolbar popup: the logins Rune Passwords has for this tab's site, from every account
// open in the app, or the one chosen at the top right. A click fills one into the page;
// "Code" copies its verification code.
const HOST = "io.brem.rune_passwords";

const ask = (query) =>
  new Promise((resolve) =>
    chrome.runtime.sendNativeMessage(HOST, query, (answer) =>
      resolve(chrome.runtime.lastError || !answer ? { error: "Rune Passwords isn't installed for this browser (run scripts/install.sh passwords)" } : answer)));

const el = (tag, cls, text) => {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
};

function note(text) {
  const n = document.getElementById("note");
  n.textContent = text;
  n.hidden = !text;
}

// Fills into the tab: the page script in each frame tries its own form. One loaded before the
// extension was installed or updated gets it injected first.
async function fill(tab, creds) {
  const message = { type: "fill", username: creds.username, password: creds.password };
  try {
    await chrome.tabs.sendMessage(tab.id, message);
  } catch {
    await chrome.scripting.executeScript({ target: { tabId: tab.id, allFrames: true }, files: ["content.js"] });
    await chrome.tabs.sendMessage(tab.id, message);
  }
}

// The account menu at the top right: All accounts, then each kept one (a locked one says so).
function accountMenu(found, account, choose) {
  const button = document.getElementById("account");
  const menu = document.getElementById("accounts");
  const current = found.accounts.find((a) => a.id === account);
  button.textContent = current ? current.email : "All accounts";
  button.hidden = found.accounts.length < 2;
  button.onclick = () => (menu.hidden = !menu.hidden);
  menu.replaceChildren();
  const item = (id, label, sub, enabled) => {
    const row = el("div", "account-item" + (id === account ? " on" : "") + (enabled ? "" : " off"));
    row.setAttribute("role", "menuitem");
    row.append(el("div", "", label));
    if (sub) row.append(el("div", "user", sub));
    if (enabled) row.onclick = () => {
      menu.hidden = true;
      choose(id);
    };
    menu.append(row);
  };
  item("", "All accounts", "Every unlocked account", true);
  for (const a of found.accounts) item(a.id, a.email, a.open ? a.host : "Locked: unlock it in Rune Passwords", a.open);
}

async function main() {
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  const url = tab?.url || "";
  let host = "";
  try {
    host = new URL(url).hostname.replace(/^www\./, "");
  } catch {}
  document.getElementById("site").textContent = host || "Rune Passwords";
  if (!/^https?:/.test(url)) return note("Nothing to fill on this page.");

  const found = await ask({ op: "logins", url });
  if (found.error) return note(found.error);
  let { account = "" } = await chrome.storage.local.get("account");
  // A chosen account since locked or gone: back to all of them.
  if (account && !found.accounts.some((a) => a.id === account && a.open)) account = "";

  const render = () => {
    accountMenu(found, account, async (id) => {
      account = id;
      await chrome.storage.local.set({ account });
      render();
    });
    const list = document.getElementById("list");
    list.replaceChildren();
    note("");
    if (found.locked) return note("Unlock Rune Passwords to fill from it.");
    const logins = found.logins.filter((l) => !account || l.account === account);
    if (!logins.length) return note(`No logins for ${host}.`);
    const several = !account && new Set(logins.map((l) => l.account)).size > 1;
    const email = (id) => found.accounts.find((a) => a.id === id)?.email || "";
    for (const login of logins) {
      const row = el("div", "login");
      row.tabIndex = 0;
      const who = el("div", "who");
      who.append(el("div", "", login.name || host), el("div", "user", several ? `${login.username} · ${email(login.account)}` : login.username));
      row.append(who);
      const choose = async () => {
        const creds = await ask({ op: "fill", account: login.account, id: login.id, url });
        if (creds.error) return note(creds.error);
        await fill(tab, creds);
        window.close();
      };
      row.addEventListener("click", choose);
      row.addEventListener("keydown", (e) => e.key === "Enter" && choose());
      if (login.code) {
        const code = el("button", "code", "Code");
        code.title = "Copy the verification code";
        code.addEventListener("click", async (e) => {
          e.stopPropagation();
          const got = await ask({ op: "code", account: login.account, id: login.id, url });
          if (got.error) return note(got.error);
          await navigator.clipboard.writeText(got.code);
          code.textContent = "Copied";
        });
        row.append(code);
      }
      list.append(row);
    }
    list.querySelector(".login")?.focus();
  };
  render();
}

main();
