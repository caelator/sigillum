export const WORKSPACE_DESTINATIONS = [
  {
    id: "overview",
    label: "Overview",
    summary: "Security posture, recommended next action, self-check, and recent audit activity.",
  },
  {
    id: "receive",
    label: "Receive",
    summary: "Private receiving, dedicated allocations, counterparties, rotation, and tracked deposits.",
  },
  {
    id: "portfolio",
    label: "Portfolio",
    summary: "Wallets, providers, treasury rollup, inventory discovery, watch lists, and risk findings.",
  },
  {
    id: "move",
    label: "Move",
    summary: "Movement policy, consolidation plans, queue review, and maintenance.",
  },
  {
    id: "vault",
    label: "Vault",
    summary: "Secrets, connection keys, compartments, recovery, hardware keys, and diagnostics.",
  },
] as const;

export type WorkspaceDestinationId = (typeof WORKSPACE_DESTINATIONS)[number]["id"];

export const WORKSPACE_DESTINATION_KEY = "sigillumWorkspaceDestinationV2";
export const DEFAULT_WORKSPACE_DESTINATION: WorkspaceDestinationId = "overview";

export function normalizeWorkspaceDestination(
  value: string | null | undefined,
): WorkspaceDestinationId {
  return WORKSPACE_DESTINATIONS.some((destination) => destination.id === value)
    ? (value as WorkspaceDestinationId)
    : DEFAULT_WORKSPACE_DESTINATION;
}

function prefersReducedMotion(): boolean {
  return Boolean(window.matchMedia?.("(prefers-reduced-motion: reduce)").matches);
}

function scrollBehavior(): ScrollBehavior {
  return prefersReducedMotion() ? "auto" : "smooth";
}

function workspaceCard(element: Element | null): HTMLElement | null {
  if (!element) return null;
  if (element instanceof HTMLElement && element.dataset.workspaceSection) return element;
  return element.closest<HTMLElement>(".card[data-workspace-section]");
}

function isCardAvailable(card: HTMLElement): boolean {
  return !card.classList.contains("hidden");
}

function readStoredDestination(): WorkspaceDestinationId {
  try {
    return normalizeWorkspaceDestination(
      window.sessionStorage.getItem(WORKSPACE_DESTINATION_KEY),
    );
  } catch (_) {
    return DEFAULT_WORKSPACE_DESTINATION;
  }
}

function storeDestination(destination: WorkspaceDestinationId): void {
  try {
    window.sessionStorage.setItem(WORKSPACE_DESTINATION_KEY, destination);
  } catch (_) {}
}

export function createWorkspaceController() {
  let activeDestination = readStoredDestination();

  function destinationCards(): HTMLElement[] {
    return Array.from(
      document.querySelectorAll<HTMLElement>("main .card[data-workspace-section]"),
    );
  }

  function firstAvailableCard(destination: WorkspaceDestinationId): HTMLElement | null {
    return (
      destinationCards().find(
        (card) =>
          card.dataset.workspaceSection === destination && isCardAvailable(card),
      ) || null
    );
  }

  function focusCard(card: HTMLElement | null): void {
    if (!card) return;
    if (!card.hasAttribute("tabindex")) card.setAttribute("tabindex", "-1");
    card.focus({ preventScroll: true });
  }

  function syncTopbar(unlocked: boolean): void {
    const title = document.getElementById("topbarTitle");
    const summary = document.getElementById("topbarSummary");
    if (!title || !summary) return;
    const destination = WORKSPACE_DESTINATIONS.find(
      (candidate) => candidate.id === activeDestination,
    );
    if (unlocked && destination) {
      title.textContent = destination.label;
      summary.textContent = destination.summary;
      return;
    }
    title.textContent = "Sigillum";
    summary.textContent = "Local treasury daemon";
  }

  function syncNavigation(unlocked: boolean): void {
    const nav = document.getElementById("sectionNav");
    if (!nav) return;
    nav.classList.toggle("hidden", !unlocked);
    nav
      .querySelectorAll<HTMLElement>("[data-action=\"selectWorkspaceSection\"]")
      .forEach((button) => {
        const selected = button.dataset.arg0 === activeDestination;
        button.classList.toggle("active", selected);
        if (selected) button.setAttribute("aria-current", "page");
        else button.removeAttribute("aria-current");
      });
  }

  function sync(): void {
    const unlocked = document.body.dataset.mode === "unlocked";
    destinationCards().forEach((card) => {
      card.classList.toggle(
        "section-hidden",
        unlocked && card.dataset.workspaceSection !== activeDestination,
      );
    });
    syncNavigation(unlocked);
    syncTopbar(unlocked);
  }

  function selectWorkspaceSection(value: unknown): void {
    const destination = normalizeWorkspaceDestination(
      typeof value === "string" ? value : null,
    );
    activeDestination = destination;
    storeDestination(destination);
    sync();
    const firstCard = firstAvailableCard(destination);
    firstCard?.scrollIntoView({ behavior: scrollBehavior(), block: "start" });
    focusCard(firstCard);
  }

  function jumpToCard(id: string): void {
    const target = document.getElementById(id);
    if (!target || target.classList.contains("hidden")) return;
    const card = workspaceCard(target);
    const destination = normalizeWorkspaceDestination(card?.dataset.workspaceSection);
    if (card && destination !== activeDestination) {
      activeDestination = destination;
      storeDestination(destination);
      sync();
    }
    requestAnimationFrame(() => {
      if (target.classList.contains("hidden") || target.classList.contains("section-hidden")) {
        return;
      }
      target.scrollIntoView({ behavior: scrollBehavior(), block: "start" });
      focusCard(target);
    });
  }

  function jumpToField(cardId: string, inputId: string): void {
    jumpToCard(cardId);
    setTimeout(() => {
      const field = document.getElementById(inputId) as HTMLElement | null;
      if (!field) return;
      field.scrollIntoView({ behavior: scrollBehavior(), block: "center" });
      field.focus({ preventScroll: true });
    }, 120);
  }

  return {
    activeDestination: () => activeDestination,
    jumpToCard,
    jumpToField,
    selectWorkspaceSection,
    sync,
  };
}
