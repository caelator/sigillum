export function byId<TElement extends HTMLElement>(
  id: string,
  expected: { new (): TElement },
): TElement | null {
  const element = document.getElementById(id);
  return element instanceof expected ? element : null;
}

export function requireById<TElement extends HTMLElement>(
  id: string,
  expected: { new (): TElement },
): TElement {
  const element = byId(id, expected);
  if (!element) {
    throw new Error(`Missing expected DOM element: ${id}`);
  }
  return element;
}

export function setHidden(element: HTMLElement | null, hidden: boolean): void {
  element?.classList.toggle("hidden", hidden);
}

export function setText(element: HTMLElement | null, value: string): void {
  if (element) {
    element.textContent = value;
  }
}

export function setTrustedHtml(element: HTMLElement | null, value: string): void {
  if (element) {
    element.innerHTML = value;
  }
}
