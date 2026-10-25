// hls.js ships no declarations for its light build, whose API is the full
// build's with some features left out.
declare module 'hls.js/light' {
	export * from 'hls.js';
	export { default } from 'hls.js';
}
