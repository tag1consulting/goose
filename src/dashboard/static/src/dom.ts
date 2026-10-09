// Element lookup. Each module looks up the elements it uses through these
// helpers, which check the element's type instead of casting it.

type ElementConstructor<T extends HTMLElement> = {
  new (): T;
  prototype: T;
};

/** The element with this id, which must exist and be a `ctor`. */
export function requireElement<T extends HTMLElement>(
  id: string,
  ctor: ElementConstructor<T>
): T {
  const el = optionalElement(id, ctor);
  if (el === null) {
    throw new Error("dashboard: missing element #" + id);
  }
  return el;
}

/** The element with this id, or null when the page has none. */
export function optionalElement<T extends HTMLElement>(
  id: string,
  ctor: ElementConstructor<T>
): T | null {
  const el = document.getElementById(id);
  if (el === null) return null;
  if (!(el instanceof ctor)) {
    throw new Error(
      "dashboard: element #" + id + " is not a " + ctor.name
    );
  }
  return el;
}
