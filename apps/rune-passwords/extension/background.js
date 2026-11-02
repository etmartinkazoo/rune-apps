// The extension's one way to the app (native messaging), the badge with how many logins the
// tab's site has, and the go-between for the fill menu that opens under a page's fields.
//
// The menu is an extension page in a frame on the page, so the page can't read the names and
// user names in it. A field's content script asks for one with a token; the menu shows the
// logins for that frame's site (from `sender.url`, never what the page says) and a pick fills
// that frame alone.

const HOST = "io.brem.rune_passwords";

const ask = (query) =>
  new Promise((resolve) =>
    chrome.runtime.sendNativeMessage(HOST, query, (answer) =>
      resolve(chrome.runtime.lastError || !answer ? { error: "Rune Passwords isn't installed for this browser" } : answer)));

// The account chosen in the popup ("" for every open one).
const chosen = async () => (await chrome.storage.local.get("account")).account || "";

const shown = (answer, account) => (answer.logins || []).filter((l) => !account || l.account === account);

// --- The badge --------------------------------------------------------------------------------

const pending = new Map();

function badge(tabId, url) {
  clearTimeout(pending.get(tabId));
  pending.set(tabId, setTimeout(async () => {
    pending.delete(tabId);
    let text = "";
    if (/^https?:/.test(url || "")) {
      const n = shown(await ask({ op: "logins", url }), await chosen()).length;
      text = n ? String(n) : "";
    }
    chrome.action.setBadgeText({ tabId, text }).catch(() => {});
  }, 250));
}

chrome.action.setBadgeBackgroundColor({ color: "#4b8ba3" });
chrome.action.setBadgeTextColor?.({ color: "#ffffff" });

chrome.tabs.onActivated.addListener(({ tabId }) => chrome.tabs.get(tabId).then((t) => badge(tabId, t.url)).catch(() => {}));
chrome.tabs.onUpdated.addListener((tabId, change, tab) => {
  if (change.url || change.status === "complete") badge(tabId, tab.url);
});
chrome.windows.onFocusChanged.addListener(() =>
  chrome.tabs.query({ active: true, lastFocusedWindow: true }).then(([t]) => t && badge(t.id, t.url)).catch(() => {}));
chrome.storage.onChanged.addListener(() =>
  chrome.tabs.query({ active: true }).then((tabs) => tabs.forEach((t) => badge(t.id, t.url))).catch(() => {}));

// --- The fill menu ----------------------------------------------------------------------------

// Menus open now, by token: the frame they fill, its page, and what kind of field asked.
const menus = new Map();

const ours = (sender) => sender.id === chrome.runtime.id && (sender.url || "").startsWith(chrome.runtime.getURL("menu.html"));

chrome.runtime.onMessage.addListener((msg, sender, reply) => {
  (async () => {
    switch (msg.type) {
      // From a page's field: are there logins to offer here?
      case "offer": {
        const answer = await ask({ op: "logins", url: sender.url });
        const logins = shown(answer, await chosen()).filter((l) => msg.kind !== "code" || l.code);
        return { show: logins.length > 0 || (answer.locked && (answer.accounts || []).length > 0) };
      }
      // From a page's field: a menu is opening, under this token.
      case "open": {
        menus.set(msg.token, { tabId: sender.tab.id, frameId: sender.frameId, url: sender.url, kind: msg.kind });
        badge(sender.tab.id, sender.tab.url);
        return {};
      }
      case "close": {
        // From the menu (Escape): its field's frame takes it away.
        const m = menus.get(msg.token);
        if (m && ours(sender)) chrome.tabs.sendMessage(m.tabId, { type: "closed", token: msg.token }, { frameId: m.frameId }).catch(() => {});
        menus.delete(msg.token);
        return {};
      }
      // From the menu: what to list.
      case "list": {
        const m = ours(sender) && menus.get(msg.token);
        if (!m) return { error: "This menu has closed" };
        const answer = await ask({ op: "logins", url: m.url });
        const account = await chosen();
        return {
          kind: m.kind,
          locked: !!answer.locked,
          error: answer.error,
          accounts: answer.accounts || [],
          logins: shown(answer, account).filter((l) => m.kind !== "code" || l.code),
        };
      }
      // From the menu: its height, for the frame around it.
      case "size": {
        const m = ours(sender) && menus.get(msg.token);
        if (m) chrome.tabs.sendMessage(m.tabId, { type: "size", token: msg.token, height: msg.height }, { frameId: m.frameId }).catch(() => {});
        return {};
      }
      // From the menu: fill this login (or its code) into the field's frame.
      case "pick": {
        const m = ours(sender) && menus.get(msg.token);
        if (!m) return { error: "This menu has closed" };
        const fill = m.kind === "code"
          ? await ask({ op: "code", account: msg.account, id: msg.id, url: m.url })
          : await ask({ op: "fill", account: msg.account, id: msg.id, url: m.url });
        if (fill.error) return fill;
        await chrome.tabs.sendMessage(m.tabId, { type: "fill", token: msg.token, ...fill }, { frameId: m.frameId }).catch(() => {});
        menus.delete(msg.token);
        return {};
      }
    }
    return {};
  })().then(reply);
  return true;
});
