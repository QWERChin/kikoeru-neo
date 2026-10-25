/** The back and forward steps until the user's settings load (contract "UserSettings"). */
export const DEFAULT_SEEK_BACK_SECONDS = 15;
export const DEFAULT_SEEK_FORWARD_SECONDS = 30;

/** The range `PUT /api/me/settings` accepts. The button icon has room for two digits. */
export const SEEK_SECONDS_MIN = 1;
export const SEEK_SECONDS_MAX = 99;

/** What the settings page offers. */
export const SEEK_CHOICES: readonly number[] = [5, 10, 15, 20, 30, 45, 60, 90];

/**
 * A step from the settings, or `fallback` when it is missing or out of
 * range. A server from before the setting sends none.
 */
export function seekSeconds(value: unknown, fallback: number): number {
	return typeof value === 'number' &&
		Number.isInteger(value) &&
		value >= SEEK_SECONDS_MIN &&
		value <= SEEK_SECONDS_MAX
		? value
		: fallback;
}

/** The settings page's choices, plus the saved step when it is not one of them. */
export function seekChoices(current: number): number[] {
	return SEEK_CHOICES.includes(current)
		? [...SEEK_CHOICES]
		: [...SEEK_CHOICES, current].sort((a, b) => a - b);
}
