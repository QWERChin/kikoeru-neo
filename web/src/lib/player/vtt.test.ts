import { describe, expect, it } from 'vitest';
import { activeCueIndex, parseVtt } from './vtt';

const SAMPLE = `\uFEFFWEBVTT
Kind: captions

NOTE converted from lrc

1
00:00:01.000 --> 00:00:04.500 align:start
<v 甲>第一句 &amp; 其他</v>

00:04.500 --> 00:08.000
第二句
还有一行

00:01:02.250 --> 00:01:05.000
<c.loud>第三句</c>

broken --> nonsense
skipped

00:00:09.000 --> 00:00:09.000
零长度被跳过
`;

describe('parseVtt', () => {
	const cues = parseVtt(SAMPLE.replace(/\n/g, '\r\n'));

	it('reads timings with and without hours, identifiers and settings', () => {
		expect(cues.map((c) => [c.start, c.end])).toEqual([
			[1, 4.5],
			[4.5, 8],
			[62.25, 65]
		]);
	});

	it('strips tags and decodes entities, keeping line breaks', () => {
		expect(cues.map((c) => c.text)).toEqual(['第一句 & 其他', '第二句\n还有一行', '第三句']);
	});

	it('sorts cues by start time', () => {
		const text = 'WEBVTT\n\n00:05.000 --> 00:06.000\nb\n\n00:01.000 --> 00:02.000\na\n';
		expect(parseVtt(text).map((c) => c.text)).toEqual(['a', 'b']);
	});

	it('accepts srt-style commas and returns nothing for an empty file', () => {
		expect(parseVtt('WEBVTT\n\n00:00:01,500 --> 00:00:02,000\nx')[0]?.start).toBe(1.5);
		expect(parseVtt('WEBVTT\n')).toEqual([]);
	});
});

describe('activeCueIndex', () => {
	const cues = parseVtt(SAMPLE);

	it('finds the cue that covers the time', () => {
		expect(activeCueIndex(cues, 1)).toBe(0);
		expect(activeCueIndex(cues, 4.4)).toBe(0);
		// The end is exclusive, so the next cue takes over exactly at 4.5.
		expect(activeCueIndex(cues, 4.5)).toBe(1);
		expect(activeCueIndex(cues, 63)).toBe(2);
	});

	it('returns -1 before the first cue and in gaps', () => {
		expect(activeCueIndex(cues, 0.5)).toBe(-1);
		expect(activeCueIndex(cues, 30)).toBe(-1);
		expect(activeCueIndex(cues, 99)).toBe(-1);
		expect(activeCueIndex([], 1)).toBe(-1);
	});

	it('prefers the most recently started cue when cues overlap', () => {
		const overlapping = [
			{ start: 0, end: 10, text: 'long' },
			{ start: 2, end: 3, text: 'short' }
		];
		expect(activeCueIndex(overlapping, 2.5)).toBe(1);
		expect(activeCueIndex(overlapping, 5)).toBe(0);
	});
});
