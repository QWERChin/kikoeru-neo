Component: frontend pages and components

Lenses: correctness

Files in scope: web/src/routes/ (all .svelte files except
(fullscreen)/player/+page.svelte), web/src/lib/components/,
web/src/lib/roots/, web/src/lib/tasks/, web/src/app.html, web/src/app.css,
web/static/theme.js, web/static/manifest.webmanifest.

Book chapters: frontend.md, api.md (admin and library flows), contract.md.

Look especially for: Svelte 5 rune misuse that shows stale or wrong data
($effect that loops or misses dependencies, $derived over non-reactive
values, props mutated locally, keyed each blocks with unstable keys);
pages that keep showing the previous work or page after client-side
navigation; forms that submit twice, lose input on error, or report success
on failure; admin UI for roots and tasks that shows wrong task state or
allows actions the backend rejects without telling the user; infinite
scroll and pagination that skip or duplicate items; {@html} or URL handling
that injects metadata text from DLsite or user reviews as markup; theme
flashing or wrong theme on first paint caused by theme.js and app.html
disagreeing.
