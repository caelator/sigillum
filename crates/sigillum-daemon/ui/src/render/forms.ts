import { esc, escAttr } from "./html";

export interface SelectOption {
  value: string | number;
  label: string;
}

export function clearFields(ids: string[]): void {
  ids.forEach((id) => {
    const el = document.getElementById(id) as
      | HTMLInputElement
      | HTMLTextAreaElement
      | null;
    if (el) el.value = "";
  });
}

export function renderEntityList<T>(
  containerId: string,
  items: T[],
  emptyMsg: string,
  renderItem: (item: T) => string,
): void {
  const el = document.getElementById(containerId);
  if (!el) return;
  if (!items.length) {
    el.innerHTML = '<p class="empty-state">' + esc(emptyMsg) + "</p>";
    return;
  }
  let html = '<ul class="entity-list">';
  items.forEach((item) => {
    html += renderItem(item);
  });
  html += "</ul>";
  el.innerHTML = html;
}

export function textValue(id: string): string {
  const el = document.getElementById(id) as HTMLInputElement | null;
  return el ? el.value.trim() : "";
}

export function optionalTextValue(id: string): string | null {
  const value = textValue(id);
  return value ? value : null;
}

export function optionalNumberValue(id: string): number | null {
  const value = textValue(id);
  if (!value) return null;
  const parsed = parseInt(value, 10);
  return Number.isFinite(parsed) ? parsed : null;
}

export function setSelectOptions(
  id: string,
  items: SelectOption[],
  placeholder?: string,
): void {
  const el = document.getElementById(id) as HTMLSelectElement | null;
  if (!el) return;
  const previous = el.value;
  let html = "";
  if (placeholder) {
    html += '<option value="">' + esc(placeholder) + "</option>";
  }
  items.forEach((item) => {
    html +=
      '<option value="' +
      escAttr(String(item.value)) +
      '">' +
      esc(item.label) +
      "</option>";
  });
  el.innerHTML = html;

  if (items.some((item) => String(item.value) === previous)) {
    el.value = previous;
  } else if (!placeholder && items[0]) {
    el.value = String(items[0].value);
  } else {
    el.value = "";
  }
}

export function showResultBox(id: string, html: string): void {
  const el = document.getElementById(id);
  if (!el) return;
  el.innerHTML = html;
  el.setAttribute("role", "status");
  el.setAttribute("aria-live", "polite");
  el.classList.remove("hidden");
}
