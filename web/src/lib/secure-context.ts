// Plain HTTP on a LAN address, such as http://192.168.1.10:8888, is a
// supported way to open the app (book: frontend.md). Browsers count such an
// origin as insecure and withhold a few APIs there, so the code that would
// use them asks these first instead of assuming HTTPS.

/** The part of `window` or a worker scope these checks read. */
export interface ContextScope {
	isSecureContext?: boolean;
	queryLocalFonts?: unknown;
}

/**
 * True when the browser says this page is not a secure context. A scope
 * that does not report it at all, such as Vitest's Node, is not treated as
 * insecure, so nothing extra shows there.
 */
export function isInsecureContext(scope: ContextScope | undefined = globalThis): boolean {
	return scope?.isSecureContext === false;
}

/**
 * JASSUB's `queryFonts` setting. Fonts installed on the device can only be
 * read through `queryLocalFonts`, which Chromium offers in a secure context
 * only and other browsers not at all. Without it JASSUB's lookup can only
 * fail, and Firefox and Safari reject its `local-fonts` permission query
 * with a TypeError, so the lookup is off.
 */
export function localFontQuery(scope: ContextScope | undefined = globalThis): 'local' | false {
	return typeof scope?.queryLocalFonts === 'function' ? 'local' : false;
}
