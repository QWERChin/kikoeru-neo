// Picks light or dark before the first paint, so a page set to one theme
// does not flash the system's first. src/lib/theme.ts applies the same rule
// once the app starts and keeps it current. theme.test.ts runs both.
(function () {
	var preference = null;
	try {
		preference = localStorage.getItem('kikoeru-neo.theme');
	} catch (e) {
		// Storage can be refused. The system's theme applies then.
	}
	var dark =
		preference === 'dark' ||
		(preference !== 'light' && matchMedia('(prefers-color-scheme: dark)').matches);
	var theme = dark ? 'dark' : 'light';
	document.documentElement.dataset.theme = theme;
	var bar = document.querySelector('meta[name="theme-color"]');
	if (bar) bar.setAttribute('content', dark ? '#000000' : '#f2f2f7');
	// The canvas follows this until app.css loads and sets color-scheme itself.
	var scheme = document.querySelector('meta[name="color-scheme"]');
	if (scheme) scheme.setAttribute('content', theme);
})();
