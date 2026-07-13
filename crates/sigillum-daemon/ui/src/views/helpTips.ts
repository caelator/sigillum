type DismissHelpTip = () => void;

const activeHelpTipByDocument = new WeakMap<Document, DismissHelpTip>();
const escapeHandlerDocuments = new WeakSet<Document>();
let nextHelpTipId = 0;

function installEscapeHandler(document: Document): void {
  if (escapeHandlerDocuments.has(document)) return;
  escapeHandlerDocuments.add(document);
  document.addEventListener("keydown", (event) => {
    if (event.key !== "Escape") return;
    const dismiss = activeHelpTipByDocument.get(document);
    if (!dismiss) return;
    event.preventDefault();
    event.stopPropagation();
    dismiss();
  });
}

function uniqueTooltipId(document: Document): string {
  let id: string;
  do {
    nextHelpTipId += 1;
    id = `help-tip-content-${nextHelpTipId}`;
  } while (document.getElementById(id));
  return id;
}

export function enhanceHelpTips(root: ParentNode = document): void {
  const triggers = Array.from(
    root.querySelectorAll<HTMLElement>(".help-tip[data-tip]"),
  );

  triggers.forEach((trigger) => {
    if (trigger.getAttribute("data-help-tip-enhanced") === "true") return;
    const content = trigger.dataset.tip?.trim();
    if (!content) return;

    const ownerDocument = trigger.ownerDocument;
    const parent = trigger.parentNode;
    if (!parent) return;
    installEscapeHandler(ownerDocument);

    const wrapper = ownerDocument.createElement("span");
    wrapper.className = "help-tip-wrapper";
    if (trigger.hasAttribute("data-tip-down")) {
      wrapper.setAttribute("data-tip-down", "");
    }
    parent.insertBefore(wrapper, trigger);
    wrapper.appendChild(trigger);

    const tooltip = ownerDocument.createElement("span");
    tooltip.id = uniqueTooltipId(ownerDocument);
    tooltip.className = "help-tip-content";
    tooltip.setAttribute("role", "tooltip");
    tooltip.textContent = content;
    wrapper.appendChild(tooltip);

    trigger.setAttribute("role", "button");
    trigger.setAttribute("aria-label", "More information");
    trigger.setAttribute("aria-describedby", tooltip.id);
    trigger.setAttribute("aria-expanded", "false");
    if (!trigger.hasAttribute("tabindex")) trigger.setAttribute("tabindex", "0");

    let hovered = false;
    let focused = false;
    let dismissed = false;

    const dismiss = () => {
      dismissed = true;
      update();
    };

    const update = () => {
      const open = !dismissed && (hovered || focused);
      trigger.setAttribute("data-help-tip-open", String(open));
      wrapper.setAttribute("data-help-tip-open", String(open));
      trigger.setAttribute("aria-expanded", String(open));
      if (open) {
        const previous = activeHelpTipByDocument.get(ownerDocument);
        if (previous && previous !== dismiss) previous();
        activeHelpTipByDocument.set(ownerDocument, dismiss);
      } else if (activeHelpTipByDocument.get(ownerDocument) === dismiss) {
        activeHelpTipByDocument.delete(ownerDocument);
      }
    };

    wrapper.addEventListener("mouseenter", () => {
      hovered = true;
      dismissed = false;
      update();
    });
    wrapper.addEventListener("mouseleave", () => {
      hovered = false;
      if (!focused) dismissed = false;
      update();
    });
    trigger.addEventListener("focus", () => {
      focused = true;
      dismissed = false;
      update();
    });
    trigger.addEventListener("blur", () => {
      focused = false;
      if (!hovered) dismissed = false;
      update();
    });
    trigger.addEventListener("click", (event) => {
      event.preventDefault();
      dismissed = false;
      focused = true;
      trigger.focus({ preventScroll: true });
      update();
    });
    trigger.addEventListener("keydown", (event) => {
      if (event.key !== "Enter" && event.key !== " ") return;
      event.preventDefault();
      dismissed = false;
      focused = true;
      update();
    });

    update();
    trigger.setAttribute("data-help-tip-enhanced", "true");
  });
}
