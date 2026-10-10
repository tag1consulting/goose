// Which response time the dashboard shows: the coordinated omission adjusted
// one when the snapshot carries it, otherwise the measured one. Imports
// nothing, so the client tests load it without the bundle.

export interface ShownTime {
  value: number;
  /** "Measured: N ms" when `value` is adjusted, or null when it is measured. */
  title: string | null;
}

/**
 * The adjusted value when there is one, with the measured value for its
 * hover; otherwise the measured value and no hover. `format` renders the
 * measured value as the cell or KPI next to it would.
 */
export function shownTime(
  measured: number,
  adjusted: number | null | undefined,
  format: (n: number) => string
): ShownTime {
  if (adjusted == null) return { value: measured, title: null };
  return { value: adjusted, title: "Measured: " + format(measured) + " ms" };
}
