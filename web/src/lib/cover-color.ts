/**
 * Hue of the placeholder cover, derived from the work id so a card keeps its
 * colour across reloads and pages. Saturation and lightness come from CSS
 * (`--cover-*` in app.css) so dark mode can dim the placeholders.
 */
export function coverHue(workId: number): number {
	// Knuth's multiplicative hash spreads consecutive ids across the hue wheel.
	return (Math.imul(workId, 2654435761) >>> 0) % 360;
}
