// A parser for text/event-stream bodies. EventSource cannot send the
// Authorization header the task stream needs, so the stream is read with
// fetch and its text is split into events here.

export interface SseEvent {
	/** `message` when the event has no `event:` line, as in EventSource. */
	event: string;
	data: string;
}

/**
 * Turns chunks of an event stream into whole events. A chunk may end in the
 * middle of a line or an event, so the unfinished part waits for the next one.
 */
export class SseParser {
	#buffer = '';
	#event = '';
	#data: string[] = [];

	push(chunk: string): SseEvent[] {
		this.#buffer += chunk;
		const events: SseEvent[] = [];
		let end: number;
		while ((end = this.#lineEnd()) >= 0) {
			const line = this.#buffer.slice(0, end);
			// A CRLF pair is one line break, so skip both characters.
			const width = this.#buffer[end] === '\r' && this.#buffer[end + 1] === '\n' ? 2 : 1;
			this.#buffer = this.#buffer.slice(end + width);
			const event = this.#line(line);
			if (event) events.push(event);
		}
		return events;
	}

	#lineEnd(): number {
		const match = /\r\n|\r|\n/.exec(this.#buffer);
		if (!match) return -1;
		// A lone CR at the end of the buffer may be the first half of CRLF.
		if (match[0] === '\r' && match.index === this.#buffer.length - 1) return -1;
		return match.index;
	}

	#line(line: string): SseEvent | null {
		if (line === '') return this.#dispatch();
		// Comment lines such as `: ping` only keep proxies from closing the stream.
		if (line.startsWith(':')) return null;
		const colon = line.indexOf(':');
		const field = colon < 0 ? line : line.slice(0, colon);
		let value = colon < 0 ? '' : line.slice(colon + 1);
		if (value.startsWith(' ')) value = value.slice(1);
		if (field === 'event') this.#event = value;
		else if (field === 'data') this.#data.push(value);
		return null;
	}

	#dispatch(): SseEvent | null {
		const event = this.#data.length > 0 ? { event: this.#event || 'message', data: this.#data.join('\n') } : null;
		this.#event = '';
		this.#data = [];
		return event;
	}
}
