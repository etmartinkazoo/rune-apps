// In every page and frame: offers the fill menu under a login field when it's focused, and
// fills what the menu (or the popup) picks. The menu itself is an extension page in a frame,
// in a closed shadow root, so the page sees neither the logins nor how many there are.
if (!window.__runePasswords) {
  window.__runePasswords = true;

  const USERISH = 'input[type=email], input[type=text], input[type=tel], input:not([type])';
  const shown = (e) => !e.disabled && !e.readOnly && e.getClientRects().length > 0 && getComputedStyle(e).visibility !== "hidden";

  // What a field takes: "password", "username", "code" (a verification code), or null.
  function kindOf(e) {
    if (!(e instanceof HTMLInputElement) || !shown(e)) return null;
    const type = (e.type || "text").toLowerCase();
    const auto = (e.autocomplete || "").toLowerCase();
    const named = `${e.name} ${e.id} ${e.placeholder} ${e.getAttribute("aria-label") || ""}`.toLowerCase();
    if (type === "password") return "password";
    if (!["text", "email", "tel", "number", ""].includes(type)) return null;
    if (auto.includes("one-time-code") || /\b(otp|totp|2fa|mfa)\b|one.?time|verification|security.?code|auth.?code/.test(named)) return "code";
    if (auto.includes("username") || auto === "email" || type === "email" || /user|login|email|account|identifier/.test(named)) {
      // A login form's: it has a password field, or this is the first step of one.
      return "username";
    }
    return null;
  }

  // Puts `value` in an input through its own setter, with the events typing fires, so pages
  // built on frameworks see it.
  function put(e, value) {
    e.focus();
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set.call(e, value);
    e.dispatchEvent(new Event("input", { bubbles: true }));
    e.dispatchEvent(new Event("change", { bubbles: true }));
  }

  // Fills a login into the form around `near` (a field, or nothing for the page's first form).
  function fillLogin(username, password, near) {
    const passwords = [...(near?.form || document).querySelectorAll("input[type=password]")].filter(shown);
    const pw = near?.type === "password" ? near : passwords[0];
    if (pw) {
      const scope = pw.form || document;
      const before = [...scope.querySelectorAll(USERISH)].filter((e) => shown(e) && kindOf(e) !== "code" && (e.compareDocumentPosition(pw) & Node.DOCUMENT_POSITION_FOLLOWING));
      const user = near && near !== pw && kindOf(near) === "username" ? near : before[before.length - 1];
      if (user && username) put(user, username);
      put(pw, password);
      return true;
    }
    // A sign-in that asks for the user name first, the password on the next step.
    const user = near && kindOf(near) === "username" ? near : [...document.querySelectorAll(USERISH)].find((e) => kindOf(e) === "username");
    if (user && username) {
      put(user, username);
      return true;
    }
    return false;
  }

  // --- The menu ---------------------------------------------------------------------------

  let menu = null; // { host, frame, field, token }

  function close() {
    if (!menu) return;
    chrome.runtime.sendMessage({ type: "close", token: menu.token }).catch(() => {});
    menu.host.remove();
    menu = null;
  }

  function place() {
    if (!menu) return;
    const r = menu.field.getBoundingClientRect();
    const width = Math.max(260, Math.min(360, r.width));
    menu.frame.style.left = `${Math.max(4, Math.min(r.left, innerWidth - width - 4))}px`;
    menu.frame.style.top = `${r.bottom + 4}px`;
    menu.frame.style.width = `${width}px`;
  }

  async function offer(field) {
    const kind = kindOf(field);
    if (!kind || menu?.field === field) return;
    close();
    let answer;
    try {
      answer = await chrome.runtime.sendMessage({ type: "offer", kind });
    } catch {
      return; // the extension was reloaded: this page's script is stale
    }
    if (!answer?.show || document.activeElement !== field) return;
    const token = crypto.randomUUID();
    const host = document.createElement("rune-passwords-menu");
    const root = host.attachShadow({ mode: "closed" });
    const frame = document.createElement("iframe");
    frame.src = chrome.runtime.getURL(`menu.html#${token}`);
    frame.setAttribute("allowtransparency", "true");
    frame.style.cssText = "position:fixed;z-index:2147483647;border:0;height:0;border-radius:10px;" +
      "box-shadow:0 8px 28px rgba(0,0,0,.22);background:transparent;color-scheme:normal;opacity:1;display:block;";
    root.append(frame);
    document.documentElement.append(host);
    menu = { host, frame, field, token };
    place();
    await chrome.runtime.sendMessage({ type: "open", token, kind: kind === "code" ? "code" : "login" }).catch(() => {});
  }

  document.addEventListener("focusin", (e) => {
    const field = e.composedPath()[0];
    if (menu && field === menu.host) return;
    if (field instanceof HTMLInputElement) offer(field);
    else close();
  }, true);
  document.addEventListener("pointerdown", (e) => {
    if (menu && e.composedPath()[0] !== menu.field && e.target !== menu.host) close();
  }, true);
  document.addEventListener("keydown", (e) => {
    if (!menu) return;
    if (e.key === "Escape") close();
    // Down from the field moves into the menu.
    if (e.key === "ArrowDown" && e.target === menu.field) {
      e.preventDefault();
      menu.frame.focus();
    }
  }, true);
  addEventListener("scroll", place, true);
  addEventListener("resize", place);

  chrome.runtime.onMessage.addListener((msg, _, reply) => {
    if (msg.type === "closed" && menu?.token === msg.token) {
      menu.host.remove();
      menu = null;
    } else if (msg.type === "size" && menu?.token === msg.token) {
      menu.frame.style.height = `${msg.height}px`;
    } else if (msg.type === "fill") {
      // From the menu: into its field. From the popup (no token): into the page's form.
      const field = msg.token ? menu?.token === msg.token && menu.field : null;
      if (msg.token && !field) return reply({ filled: false });
      let filled;
      if (msg.code !== undefined) {
        put(field, msg.code);
        filled = true;
      } else {
        filled = fillLogin(msg.username, msg.password, field || null);
      }
      close();
      reply({ filled });
    }
  });
}
