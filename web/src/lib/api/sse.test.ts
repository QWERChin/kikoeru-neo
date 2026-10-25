import { describe, expect, it } from 'vitest';
import { SseParser } from './sse';

describe('SseParser', () => {
	it('joins events split across chunks and skips comments', () => {
		const parser = new SseParser();
		expect(parser.push('event: snap')).toEqual([]);
		expect(parser.push('shot\ndata: {"a":')).toEqual([]);
		expect(parser.push('1}\n\n: ping\n\n')).toEqual([{ event: 'snapshot', data: '{"a":1}' }]);
	});

	it('handles CRLF, a CR split from its LF and multi-line data', () => {
		const parser = new SseParser();
		expect(parser.push('data: one\r')).toEqual([]);
		expect(parser.push('\ndata:two\r\n\r\n')).toEqual([{ event: 'message', data: 'one\ntwo' }]);
	});

	it('does not dispatch an event without data', () => {
		const parser = new SseParser();
		expect(parser.push('event: snapshot\n\n')).toEqual([]);
		// The event name does not leak into the next event.
		expect(parser.push('data: x\n\n')).toEqual([{ event: 'message', data: 'x' }]);
	});
});
