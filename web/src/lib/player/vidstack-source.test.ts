import { afterEach, describe, expect, it, vi } from 'vitest';
import { SourceSequence, isHlsType, isSourceElementFailure, vidstackSource } from './vidstack-source';

describe('vidstackSource', () => {
	it('passes types Vidstack knows through unchanged', () => {
		expect(vidstackSource('/api/tracks/1/stream?token=t', 'audio/mpeg')).toEqual({
			src: '/api/tracks/1/stream?token=t',
			type: 'audio/mpeg'
		});
		expect(vidstackSource('/a', 'video/mp4').src).toBe('/a');
	});

	it('passes an HLS playlist through so Vidstack picks its HLS path', () => {
		const url = '/api/tracks/201/hls/index.m3u8?token=t';
		expect(vidstackSource(url, 'application/vnd.apple.mpegurl')).toEqual({
			src: url,
			type: 'application/vnd.apple.mpegurl'
		});
		expect(isHlsType('application/vnd.apple.mpegurl')).toBe(true);
		expect(isHlsType('video/mp4')).toBe(false);
	});

	it('adds an extension fragment for audio and video types it does not know', () => {
		expect(vidstackSource('/api/tracks/1/stream?token=t', 'audio/wav')).toEqual({
			src: '/api/tracks/1/stream?token=t#media.m4a',
			type: 'audio/wav'
		});
		expect(vidstackSource('/b', 'video/x-matroska').src).toBe('/b#media.mp4');
		expect(vidstackSource('/c', 'audio/aac; codecs=mp4a').src).toBe('/c#media.m4a');
	});
});

describe('SourceSequence', () => {
	it('makes a repeated URL look new so Vidstack loads it again', () => {
		const sources = new SourceSequence();
		expect(sources.next('/a', 'audio/mpeg').src).toBe('/a');
		expect(sources.next('/a', 'audio/mpeg').src).toBe('/a#reload-1');
		expect(sources.next('/a', 'audio/mpeg').src).toBe('/a#reload-2');
		expect(sources.next('/b', 'audio/wav').src).toBe('/b#media.m4a');
		expect(sources.next('/b', 'audio/wav').src).toBe('/b#reload-1-media.m4a');
		sources.reset();
		expect(sources.next('/b', 'audio/wav').src).toBe('/b#media.m4a');
	});
});

describe('isSourceElementFailure', () => {
	// The unit tests run without a DOM, so the element classes are stand-ins.
	class FakeMedia {}
	class FakeSource {
		constructor(readonly parentElement: unknown) {}
	}

	afterEach(() => {
		vi.unstubAllGlobals();
	});

	it('recognises a failed <source> inside a media element and nothing else', () => {
		vi.stubGlobal('HTMLMediaElement', FakeMedia);
		vi.stubGlobal('HTMLSourceElement', FakeSource);
		expect(isSourceElementFailure(new FakeSource(new FakeMedia()) as unknown as EventTarget)).toBe(true);
		expect(isSourceElementFailure(new FakeSource(null) as unknown as EventTarget)).toBe(false);
		expect(isSourceElementFailure(new FakeMedia() as unknown as EventTarget)).toBe(false);
		expect(isSourceElementFailure(null)).toBe(false);
	});

	it('is false where the DOM classes do not exist', () => {
		expect(isSourceElementFailure({} as EventTarget)).toBe(false);
	});
});
