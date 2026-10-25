// Shared by the mock transport (browser) and the mock media middleware
// (dev server), so both agree on ids and tokens without importing each other.

/** Every mock video lasts this long, in the track list and in the files. */
export const MOCK_VIDEO_SECONDS = 60;

/** The direct-play mp4 that mock mode adds next to the sample video track. */
export const MOCK_TEASER_TRACK_ID = 202;

/** The HLS set id of the sample playlist, so mock segment URLs look real. */
export const MOCK_HLS_SET =
	'3f2a9c51e07b84d6a1c3e5f7092b4d6e8f0a1c3e5b7d9f1a2c4e6a8b0d2f4a6c-1791700300123456789';

/**
 * The first media token is the sample's. Each refresh in the same page gets
 * a new one, so a test can expire the old token and see the player switch.
 */
export function mockMediaToken(sample: string, refreshes: number): string {
	return refreshes === 0 ? sample : `${sample}-r${refreshes}`;
}

export function isMockMediaToken(token: string, sample: string): boolean {
	return (
		token === sample || (token.startsWith(sample) && /^-r\d+$/.test(token.slice(sample.length)))
	);
}
