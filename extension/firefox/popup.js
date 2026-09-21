// ポップアップ。直近の記録結果と、接続確認だけを見せる。
// 一覧や検索は GUI (fo-gui) の仕事なので、ここには持たせない。

const api = typeof browser !== "undefined" ? browser : chrome;
const el = document.getElementById("state");

function render(status) {
  const state = (status && status.state) || "idle";
  el.className = "state " + state;
  el.textContent = (status && status.message) || "まだ記録していません";
}

api.runtime.sendMessage({ type: "getStatus" }, render);

document.getElementById("ping").addEventListener("click", () => {
  el.className = "state idle";
  el.textContent = "確認中…";
  api.runtime.sendMessage({ type: "ping" }, (res) => {
    if (res && res.ok) {
      render({ state: "ok", message: "ネイティブホストに接続できました" });
    } else {
      render({
        state: "error",
        message: (res && res.error) || "接続できません。fo host install と fo-daemon を確認してください。",
      });
    }
  });
});
