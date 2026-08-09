/**
 * Element presentation, keyed by the game's internal enum name.
 *
 * Keyed by `ElementView.id` rather than `name` on purpose: the label is
 * localized, so a Japanese or German extraction would miss every colour if the
 * lookup went through display text. The nine ids are what
 * `EPalElementType` defines, and they do not change with language.
 */
const ELEMENT_COLORS: Record<string, string> = {
  Normal: "#b3b8c4",
  Fire: "#f0703c",
  Water: "#4aa3e0",
  Leaf: "#5ec14e",
  Electricity: "#f2c53d",
  Ice: "#63cbd6",
  Earth: "#c08a4a",
  Dark: "#a678d8",
  Dragon: "#7d7ce0",
};

/** Fallback for an element the game adds after this build. */
const UNKNOWN_ELEMENT = "#8d97ab";

export function elementColor(id: string): string {
  return ELEMENT_COLORS[id] ?? UNKNOWN_ELEMENT;
}
