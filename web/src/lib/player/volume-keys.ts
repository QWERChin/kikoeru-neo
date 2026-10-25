import { player } from './player.svelte';
import { volumeKeyStep } from './volume';

/**
 * Elements whose arrow keys already mean something: sliders (the progress
 * and volume sliders move themselves), text fields, and the scrolling
 * subtitle and queue panels, which mark themselves with `data-own-arrows`
 * when they are not a tab panel.
 */
const OWNS_ARROW_KEYS =
	'input, textarea, select, [contenteditable], [role="slider"], [role="tabpanel"], [data-own-arrows]';

/** Up and down change the volume while focus is in the player. */
export function onVolumeKey(event: KeyboardEvent): void {
	if (event.defaultPrevented || !player.active || !player.volumeAdjustable) return;
	const step = volumeKeyStep(event);
	if (step === null) return;
	if (event.target instanceof Element && event.target.closest(OWNS_ARROW_KEYS)) return;
	event.preventDefault();
	player.changeVolume(step);
}
