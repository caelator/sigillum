export interface LockUnconfirmedRenderer {
  clear: () => void;
  render: (canRetry: boolean, canAcknowledge: boolean) => void;
}

export function createLockUnconfirmedRenderer(): LockUnconfirmedRenderer {
  function clear(): void {
    document.body.dataset.lockUnconfirmed = "false";
    document.querySelector(".app-shell")?.removeAttribute("inert");
    document.getElementById("lockUnconfirmedAlert")?.remove();
  }

  function render(canRetry: boolean, canAcknowledge: boolean): void {
    document.body.dataset.lockUnconfirmed = "true";
    document.querySelector(".app-shell")?.setAttribute("inert", "");
    let alert = document.getElementById("lockUnconfirmedAlert");
    if (!alert) {
      alert = document.createElement("section");
      alert.id = "lockUnconfirmedAlert";
      alert.className = "lock-unconfirmed-alert";
      alert.setAttribute("role", "alert");
      alert.setAttribute("aria-live", "assertive");
      document.body.appendChild(alert);
    }
    alert.replaceChildren();

    const title = document.createElement("h2");
    title.textContent = "LOCK NOT CONFIRMED";
    const explanation = document.createElement("p");
    explanation.textContent = canRetry
      ? "The browser lost confirmation while Sigillum may still hold unlocked keys. Normal requests are blocked. Retry Lock uses isolated lock-only authority that no other action can access."
      : "The browser lost confirmation while Sigillum may still hold unlocked keys. Normal requests are blocked, and this tab no longer has retry authority."
    const guidance = document.createElement("p");
    guidance.textContent =
      "Use Retry Lock when available, or Quit from the Sigillum desktop tray. If you launched the daemon from a terminal, stop that daemon process before continuing.";
    const retry = document.createElement("button");
    retry.type = "button";
    retry.className = "btn-danger";
    retry.dataset.action = "retryUnconfirmedLock";
    retry.textContent = canRetry ? "Retry Lock" : "Retry authority erased";
    retry.disabled = !canRetry;
    alert.append(title, explanation, guidance, retry);
    if (canAcknowledge) {
      const acknowledge = document.createElement("button");
      acknowledge.type = "button";
      acknowledge.className = "btn-ghost";
      acknowledge.dataset.action = "acknowledgeDaemonRestart";
      acknowledge.textContent = "I stopped or restarted the daemon";
      alert.appendChild(acknowledge);
    }
    (canRetry ? retry : alert.querySelector<HTMLButtonElement>("button:not(:disabled)"))
      ?.focus?.();
  }

  return { clear, render };
}
