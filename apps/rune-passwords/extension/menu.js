// The menu under a page's field: the logins (or verification codes) Rune Passwords has for
// that frame's site. A click or Return fills it; arrow keys move; Escape closes.
const token = location.hash.slice(1);

const size = () =>
  chrome.runtime.sendMessage({ type: "size", token, height: Math.min(document.body.scrollHeight, 320) }).catch(() => {});

function note(text) {
  const n = document.getElementById("note");
  n.textContent = text;
  n.hidden = false;
  size();
}

async function main() {
  const got = await chrome.runtime.sendMessage({ type: "list", token });
  if (got.error) return note(got.error);
  if (!got.logins.length) {
    return note(got.locked ? "Unlock Rune Passwords to fill from it." : "Nothing to fill here.");
  }
  const several = new Set(got.logins.map((l) => l.account)).size > 1;
  const email = (id) => got.accounts.find((a) => a.id === id)?.email || "";
  const list = document.getElementById("list");
  for (const login of got.logins) {
    const row = document.createElement("div");
    row.className = "login";
    row.tabIndex = 0;
    const who = document.createElement("div");
    who.className = "who";
    const name = document.createElement("div");
    name.textContent = got.kind === "code" ? `Code for ${login.name}` : login.name;
    const user = document.createElement("div");
    user.className = "user";
    user.textContent = several ? `${login.username} · ${email(login.account)}` : login.username;
    who.append(name, user);
    row.append(who);
    const pick = async () => {
      const r = await chrome.runtime.sendMessage({ type: "pick", token, account: login.account, id: login.id });
      if (r?.error) note(r.error);
    };
    row.addEventListener("click", pick);
    row.addEventListener("keydown", (e) => {
      if (e.key === "Enter") pick();
      if (e.key === "ArrowDown") row.nextElementSibling?.focus();
      if (e.key === "ArrowUp") row.previousElementSibling?.focus();
    });
    list.append(row);
  }
  size();
}

// Focused from the field (its Down key): onto the first login.
addEventListener("focus", () => document.querySelector(".login")?.focus());
addEventListener("keydown", (e) => {
  if (e.key === "Escape") chrome.runtime.sendMessage({ type: "close", token }).catch(() => {});
});

main();
