export interface Cue {
	/** Seconds. */
	start: number;
	end: number;
	text: string;
}

const TIMING = /^\s*((?:\d+:)?\d{1,2}:\d{2}[.,]\d{1,3})\s+-->\s+((?:\d+:)?\d{1,2}:\d{2}[.,]\d{1,3})/;

function parseTimestamp(value: string): number {
	const [clock, fraction = '0'] = value.replace(',', '.').split('.');
	const units = (clock ?? '').split(':').map(Number);
	const seconds = units.reduce((total, unit) => total * 60 + unit, 0);
	return seconds + Number(`0.${fraction}`);
}

const ENTITIES: Record<string, string> = {
	'&amp;': '&',
	'&lt;': '<',
	'&gt;': '>',
	'&nbsp;': '\u00a0',
	'&lrm;': '\u200e',
	'&rlm;': '\u200f'
};

// The panel shows plain text, so voice, class and karaoke timestamp tags are
// dropped rather than rendered.
function cleanText(lines: string[]): string {
	return lines
		.join('\n')
		.replace(/<[^>]*>/g, '')
		.replace(/&(?:amp|lt|gt|nbsp|lrm|rlm);/g, (entity) => ENTITIES[entity] ?? entity)
		.trim();
}

/**
 * Parses the cues of a WebVTT file. The server converts lrc and srt into
 * WebVTT, so only the parts of the format those produce matter: optional cue
 * identifiers, timings with settings, and multi-line text. NOTE, STYLE and
 * REGION blocks and malformed cues are skipped instead of failing the file.
 */
export function parseVtt(text: string): Cue[] {
	const blocks = text
		.replace(/^\uFEFF/, '')
		.replace(/\r\n?/g, '\n')
		.split(/\n{2,}/);
	const cues: Cue[] = [];
	for (const block of blocks) {
		const lines = block.replace(/^\n+/, '').split('\n');
		const timingIndex = lines.findIndex((line) => line.includes('-->'));
		if (timingIndex === -1 || timingIndex > 1) continue;
		const match = TIMING.exec(lines[timingIndex] ?? '');
		if (!match?.[1] || !match[2]) continue;
		const start = parseTimestamp(match[1]);
		const end = parseTimestamp(match[2]);
		const body = cleanText(lines.slice(timingIndex + 1));
		if (end <= start || body === '') continue;
		cues.push({ start, end, text: body });
	}
	// Stable sort: cues with the same start keep their file order.
	return cues.sort((a, b) => a.start - b.start);
}

/**
 * Index of the cue to highlight at `time` seconds, or -1 between cues. When
 * cues overlap, the one that started last wins, because it is the line that
 * was just spoken.
 */
export function activeCueIndex(cues: readonly Cue[], time: number): number {
	let low = 0;
	let high = cues.length - 1;
	let last = -1;
	while (low <= high) {
		const mid = (low + high) >> 1;
		if ((cues[mid]?.start ?? Infinity) <= time) {
			last = mid;
			low = mid + 1;
		} else {
			high = mid - 1;
		}
	}
	for (let i = last; i >= 0; i--) {
		const cue = cues[i];
		if (cue && time < cue.end) return i;
	}
	return -1;
}
