/**
 * Sigillum Pay Widget — Drop-in crypto payment modal.
 *
 * Usage:
 *   <script src="https://your-gateway/widget/widget.js"></script>
 *   <script>
 *     SigillumPay.configure({ gateway: 'https://your-gateway', apiKey: 'sgw_...' });
 *     document.getElementById('pay-button').addEventListener('click', () => {
 *       SigillumPay.open({ amount: '0x...', chainId: 1 });
 *     });
 *   </script>
 *   <button id="pay-button">Pay</button>
 */
(function () {
  "use strict";

  const POLL_INTERVAL_MS = 5000;
  let config = { gateway: "", apiKey: "" };
  let modal = null;
  let pollTimer = null;
  let activePaymentId = null;

  const SigillumPay = {
    configure(opts) {
      config.gateway = (opts.gateway || "").replace(/\/$/, "");
      config.apiKey = opts.apiKey || "";
    },

    async open(opts = {}) {
      if (!config.gateway || !config.apiKey) {
        console.error("SigillumPay: call configure() first");
        return;
      }

      showModal("Creating payment...", "loading");

      try {
        const res = await fetch(`${config.gateway}/api/v1/payments`, {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
            Authorization: `Bearer ${config.apiKey}`,
          },
          body: JSON.stringify({
            amount_wei: opts.amount || opts.amount_wei || "0x0",
            chain_id: opts.chainId || opts.chain_id || 1,
            token_address: opts.tokenAddress || opts.token_address || null,
            metadata: opts.metadata || {},
          }),
        });

        if (!res.ok) {
          const err = await res.json().catch(() => ({}));
          showModal(err.error || "Failed to create payment", "error");
          return;
        }

        const payment = await res.json();
        showPaymentDetails(payment, opts);
      } catch (e) {
        showModal("Network error: " + e.message, "error");
      }
    },

    close() {
      clearPollState();
      if (!modal) return;
      modal.remove();
      modal = null;
    },
  };

  function clearPollState() {
    activePaymentId = null;
    if (!pollTimer) return;
    clearTimeout(pollTimer);
    pollTimer = null;
  }

  function createModalShell() {
    const overlay = document.createElement("div");
    overlay.className = "sgw-overlay";
    overlay.addEventListener("click", (event) => {
      if (event.target === overlay) SigillumPay.close();
    });

    const card = document.createElement("div");
    card.className = "sgw-modal";

    const close = document.createElement("button");
    close.type = "button";
    close.className = "sgw-close";
    close.textContent = "x";
    close.setAttribute("aria-label", "Close payment modal");
    close.addEventListener("click", () => SigillumPay.close());

    card.appendChild(close);
    overlay.appendChild(card);
    return { overlay, card };
  }

  function showModal(message, state) {
    ensureStyles();
    SigillumPay.close();

    const shell = createModalShell();
    const content = document.createElement("div");
    content.className = `sgw-content sgw-${state}`;
    if (state === "loading") {
      const spinner = document.createElement("div");
      spinner.className = "sgw-spinner";
      content.appendChild(spinner);
    }
    const text = document.createElement("p");
    text.textContent = String(message || "");
    content.appendChild(text);

    shell.card.appendChild(content);
    document.body.appendChild(shell.overlay);
    modal = shell.overlay;
  }

  function showPaymentDetails(payment, opts) {
    ensureStyles();
    SigillumPay.close();

    const addr = String(payment.stealth_address || "");
    const shortAddr = addr.length > 14 ? `${addr.slice(0, 8)}...${addr.slice(-6)}` : addr;
    const shell = createModalShell();
    const content = document.createElement("div");
    content.className = "sgw-content sgw-payment";

    const header = document.createElement("div");
    header.className = "sgw-header";
    const icon = document.createElement("div");
    icon.className = "sgw-icon";
    icon.textContent = "Payment";
    const title = document.createElement("h2");
    title.textContent = "Send Payment";
    header.appendChild(icon);
    header.appendChild(title);

    const amount = document.createElement("div");
    amount.className = "sgw-amount";
    amount.textContent = `${formatAmount(payment.amount_wei)} ${payment.token_address ? "tokens" : "ETH"}`;

    const chain = document.createElement("div");
    chain.className = "sgw-chain";
    chain.textContent = `Chain ID: ${payment.chain_id}`;

    const addressBox = document.createElement("div");
    addressBox.className = "sgw-address-box";
    const label = document.createElement("label");
    label.textContent = "Send to this address:";
    const address = document.createElement("div");
    address.className = "sgw-address";
    address.title = addr;
    address.textContent = shortAddr;
    const copy = document.createElement("button");
    copy.type = "button";
    copy.className = "sgw-copy";
    copy.textContent = "Copy";
    copy.addEventListener("click", async () => {
      try {
        await navigator.clipboard.writeText(addr);
        copy.textContent = "Copied";
        setTimeout(() => {
          if (copy.isConnected) copy.textContent = "Copy";
        }, 1500);
      } catch (error) {
        console.error("SigillumPay: clipboard copy failed", error);
        copy.textContent = "Unavailable";
      }
    });
    addressBox.appendChild(label);
    addressBox.appendChild(address);
    addressBox.appendChild(copy);

    const status = document.createElement("div");
    status.className = "sgw-status";
    setStatus(status, "Waiting for payment...", { pending: true });

    const expires = document.createElement("div");
    expires.className = "sgw-expires";
    expires.textContent = `Expires: ${formatExpiry(payment.expires_at)}`;

    content.appendChild(header);
    content.appendChild(amount);
    content.appendChild(chain);
    content.appendChild(addressBox);
    content.appendChild(status);
    content.appendChild(expires);
    shell.card.appendChild(content);
    document.body.appendChild(shell.overlay);
    modal = shell.overlay;

    // Start polling for confirmation
    activePaymentId = payment.payment_id;
    void pollPayment(payment.payment_id, status, opts);
  }

  function setStatus(statusEl, message, options = {}) {
    statusEl.replaceChildren();
    if (options.pending) {
      const spinner = document.createElement("div");
      spinner.className = "sgw-spinner-sm";
      statusEl.appendChild(spinner);
    }
    const text = document.createElement("span");
    if (options.className) text.className = options.className;
    text.textContent = message;
    statusEl.appendChild(text);
  }

  function formatExpiry(value) {
    const parsed = value ? new Date(`${value}Z`) : null;
    return parsed && !Number.isNaN(parsed.getTime())
      ? parsed.toLocaleTimeString()
      : String(value || "-");
  }

  function schedulePoll(paymentId, statusEl, opts) {
    clearPollState();
    activePaymentId = paymentId;
    pollTimer = setTimeout(() => {
      void pollPayment(paymentId, statusEl, opts);
    }, POLL_INTERVAL_MS);
  }

  async function pollPayment(paymentId, statusEl, opts) {
    if (!statusEl || !statusEl.isConnected || activePaymentId !== paymentId) return;

    try {
      const res = await fetch(
        `${config.gateway}/api/v1/payments/${paymentId}`,
        { headers: { Authorization: `Bearer ${config.apiKey}` } }
      );
      if (!res.ok) return;

      const data = await res.json();

      if (data.status === "confirmed" || data.status === "sweeping") {
        setStatus(statusEl, "Payment confirmed!", { className: "sgw-confirmed" });
        if (opts.onSuccess) opts.onSuccess(data);
        if (opts.successUrl) setTimeout(() => (window.location.href = opts.successUrl), 2000);
        return;
      }

      if (data.status === "swept") {
        setStatus(statusEl, "Payment complete!", { className: "sgw-confirmed" });
        if (opts.onSuccess) opts.onSuccess(data);
        if (opts.successUrl) setTimeout(() => (window.location.href = opts.successUrl), 1500);
        return;
      }

      if (data.status === "expired" || data.status === "cancelled") {
        setStatus(statusEl, `Payment ${data.status}`, { className: "sgw-expired" });
        if (opts.onError) opts.onError(data);
        return;
      }
    } catch (e) {
      /* ignore polling errors */
    }

    schedulePoll(paymentId, statusEl, opts);
  }

  function formatAmount(weiHex) {
    try {
      const wei = BigInt(weiHex);
      const eth = Number(wei) / 1e18;
      return eth.toFixed(eth < 0.001 ? 6 : 4);
    } catch {
      return weiHex;
    }
  }

  function ensureStyles() {
    if (document.getElementById("sgw-styles")) return;
    const link = document.createElement("link");
    link.id = "sgw-styles";
    link.rel = "stylesheet";
    link.href = `${config.gateway}/widget/widget.css`;
    document.head.appendChild(link);
  }

  // Export globally
  window.SigillumPay = SigillumPay;
})();
