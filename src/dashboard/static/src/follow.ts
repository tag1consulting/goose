// Fields that follow the server unless being edited.
//
// An input shows the server's value until the user changes it. The edit
// then holds until its apply succeeds and a snapshot reports the applied
// value, or until another client changes that value, when the field shows
// the server's value and says it changed elsewhere. Snapshots can trail a
// control reply, so until a snapshot reports what this tab sent, a snapshot
// still showing the value from before the first apply, or any value this
// tab sent since, is not a change from elsewhere. This is the fix for a
// Target field that kept the last value this tab applied and ignored every
// change made from telnet, WebSocket or another tab.

/** A text or number input, or a select: the fields that follow the server. */
export type FollowedElement = HTMLInputElement | HTMLSelectElement;

/** Whether two values of a followed setting are the same setting. */
export type Same = (a: string, b: string) => boolean;

/**
 * A value that follows the server unless being edited, as text, wherever it
 * is shown: one input or select (FollowedField), or a number and its unit
 * followed as one setting. Values are compared with `same`, so text the
 * server writes another way than the user typed it is still what was sent.
 */
export abstract class FollowedValue {
  /** The server's value as of the last snapshot or accepted reply. */
  protected server: string | null = null;
  /** The server's value before this tab's first unconfirmed change. */
  private baseline: string | null = null;
  /** The user's unapplied text, or null while following the server. */
  private edit: string | null = null;
  /** Values this tab sent that no snapshot has reported yet, oldest first. */
  private awaiting: string[] = [];

  /**
   * @param label its name in notes, such as "Target"
   * @param note says something once through the status line
   * @param same whether two values are the same setting; equal text by default
   */
  constructor(
    readonly label: string,
    private readonly note: (text: string) => void,
    private readonly same: Same = (a, b) => a === b
  ) {}

  /** Show `text`. */
  protected abstract show(text: string): void;

  /** `value` as a note names it. */
  protected describe(value: string): string {
    return value === "" ? "none" : value;
  }

  /** Focus left the field: typing the server's own value back is not an edit. */
  protected blurred(): void {
    if (
      this.edit !== null &&
      this.awaiting.length === 0 &&
      this.server !== null &&
      this.same(this.edit, this.server)
    ) {
      this.edit = null;
    }
  }

  /** Whether the user has an unapplied edit. */
  pending(): boolean {
    return this.edit !== null;
  }

  /** Whether the unapplied edit is `value`, as when the server has it. */
  editIs(value: string): boolean {
    return this.edit !== null && this.same(this.edit, value);
  }

  /** The user (or a button acting for them) set the field to `text`. */
  edited(text: string): void {
    if (this.edit === null && this.awaiting.length === 0) {
      this.baseline = this.server;
    }
    this.edit = text;
    this.show(text);
  }

  /** `text` is being applied. */
  sending(text: string): void {
    if (this.edit === null && this.awaiting.length === 0) {
      this.baseline = this.server;
    }
    this.edit = text;
    this.awaiting.push(text);
    this.show(text);
  }

  /** The server accepted `text`; a snapshot still has to report it. */
  accepted(text: string): void {
    if (this.edit === text) this.edit = null;
    this.show(text);
  }

  /** The server refused `text`, or never answered: the edit stays. */
  refused(text: string): void {
    this.awaiting = this.awaiting.filter((sent) => sent !== text);
  }

  /** Discard an unapplied edit, saying why once; the field follows again. */
  discard(reason: string): void {
    if (this.edit === null) return;
    this.revert();
    this.note(reason + "; unapplied " + this.label + " edit discarded");
  }

  /** Drop an unapplied edit without a word, as Discard edits does. */
  revert(): void {
    this.edit = null;
    this.awaiting = [];
    this.baseline = this.server;
    this.show(this.server === null ? "" : this.server);
  }

  /** A snapshot's value; `null` says nothing about this field. */
  fromServer(value: string | null): void {
    if (value === null) return;
    this.server = value;
    const same = (other: string | null): boolean => other !== null && this.same(other, value);
    if (this.awaiting.length > 0) {
      if (same(this.awaiting[this.awaiting.length - 1])) {
        this.awaiting = [];
        this.baseline = value;
        if (this.edit === null || same(this.edit)) {
          this.edit = null;
          this.show(value);
        }
        return;
      }
      // A snapshot from before this tab's change reached the server.
      if (same(this.baseline) || this.awaiting.some(same)) return;
      this.awaiting = [];
      this.changedElsewhere(value);
      return;
    }
    if (this.edit !== null) {
      // The server has the user's own value, as when an apply timed out
      // but landed: the edit is done, not changed elsewhere.
      if (same(this.edit)) {
        this.edit = null;
        this.baseline = value;
        this.show(value);
        return;
      }
      if (!same(this.baseline)) this.changedElsewhere(value);
      return;
    }
    this.baseline = value;
    this.show(value);
  }

  private changedElsewhere(value: string): void {
    this.edit = null;
    this.baseline = value;
    this.show(value);
    this.note(this.label + " changed elsewhere to " + this.describe(value));
  }
}

/**
 * One input or select that follows a server value. A select has no read
 * only state, so while it says it is unavailable (`aria-disabled`) a change
 * puts back the value it showed.
 */
export class FollowedField extends FollowedValue {
  /** The value the field last showed, which a refused select change keeps. */
  private shown = "";

  /**
   * @param input the field
   * @param label its name in notes, such as "Target"
   * @param note says something once through the status line
   * @param same whether two values are the same setting; equal text by default
   */
  constructor(
    private readonly input: FollowedElement,
    label: string,
    note: (text: string) => void,
    same?: Same
  ) {
    super(label, note, same);
    this.shown = input.value;
    const select = input instanceof HTMLSelectElement;
    input.addEventListener(select ? "change" : "input", () => {
      if (select && input.getAttribute("aria-disabled") === "true") {
        input.value = this.shown;
        return;
      }
      this.edited(input.value);
    });
    input.addEventListener("blur", () => {
      this.blurred();
    });
  }

  /** The value the field shows. */
  text(): string {
    return this.input.value;
  }

  protected show(text: string): void {
    this.shown = text;
    if (this.input.value !== text) this.input.value = text;
  }
}

/** Add `id` to the element's `aria-describedby`, keeping the others. */
export function addDescribedBy(el: HTMLElement, id: string): void {
  const ids = (el.getAttribute("aria-describedby") || "").split(/\s+/).filter(Boolean);
  if (ids.indexOf(id) < 0) ids.push(id);
  el.setAttribute("aria-describedby", ids.join(" "));
}

/** Remove `id` from the element's `aria-describedby`, keeping the others. */
export function removeDescribedBy(el: HTMLElement, id: string): void {
  const ids = (el.getAttribute("aria-describedby") || "")
    .split(/\s+/)
    .filter((other) => other !== "" && other !== id);
  if (ids.length) {
    el.setAttribute("aria-describedby", ids.join(" "));
  } else {
    el.removeAttribute("aria-describedby");
  }
}

/**
 * Make a text or number input read only, or editable again. Read only, not
 * disabled: it keeps focus and stays readable, and says it is unavailable. A
 * select only says so; its FollowedField puts back any change.
 */
export function setReadOnly(input: FollowedElement, readOnly: boolean): void {
  if (input instanceof HTMLInputElement) input.readOnly = readOnly;
  if (readOnly) {
    input.setAttribute("aria-disabled", "true");
  } else {
    input.removeAttribute("aria-disabled");
  }
}
