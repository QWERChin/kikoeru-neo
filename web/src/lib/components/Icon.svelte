<script lang="ts" module>
	// Stroke paths on a 24x24 grid, drawn in currentColor so icons follow the
	// text colour of their button or link.
	const PATHS = {
		library: 'M4 5h4v14H4zM10 5h4v14h-4zM16.5 5.5l3.5 1-3.5 12.5-3.5-1z',
		search: 'M10.5 4a6.5 6.5 0 1 0 0 13 6.5 6.5 0 0 0 0-13zM15.5 15.5 20 20',
		heart:
			'M12 20s-7-4.35-7-10a4 4 0 0 1 7-2.65A4 4 0 0 1 19 10c0 5.65-7 10-7 10z',
		clock: 'M12 3.5a8.5 8.5 0 1 0 0 17 8.5 8.5 0 0 0 0-17zM12 7.5V12l3 2',
		settings:
			'M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6zM19 12a7 7 0 0 0-.1-1.2l2-1.5-2-3.4-2.3.9a7 7 0 0 0-2.1-1.2L14 3h-4l-.5 2.6a7 7 0 0 0-2.1 1.2l-2.3-.9-2 3.4 2 1.5a7 7 0 0 0 0 2.4l-2 1.5 2 3.4 2.3-.9a7 7 0 0 0 2.1 1.2L10 21h4l.5-2.6a7 7 0 0 0 2.1-1.2l2.3.9 2-3.4-2-1.5c.07-.4.1-.8.1-1.2z',
		people:
			'M9 11a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7zM3 20a6 6 0 0 1 12 0M16 4.5a3.5 3.5 0 0 1 0 6.5M18 14.5a6 6 0 0 1 3 5.5',
		drive: 'M4 6h16v5H4zM4 13h16v5H4zM7.5 8.5h.01M7.5 15.5h.01',
		cloud: 'M7 18a4 4 0 0 1-.6-7.95A6 6 0 0 1 18 9a4.5 4.5 0 0 1-.5 9z',
		music: 'M9 18V6l10-2v12M9 18a2.5 2.5 0 1 1-5 0 2.5 2.5 0 0 1 5 0zM19 16a2.5 2.5 0 1 1-5 0 2.5 2.5 0 0 1 5 0z',
		check: 'M5 12.5l4.5 4.5L19 7.5',
		plus: 'M12 5v14M5 12h14',
		info: 'M12 3.5a8.5 8.5 0 1 0 0 17 8.5 8.5 0 0 0 0-17zM12 11v5.5M12 7.75h.01',
		chevronLeft: 'M15 5l-7 7 7 7',
		chevronRight: 'M9 5l7 7-7 7',
		logout: 'M14 4h4a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2h-4M10 16l-4-4 4-4M6 12h10',
		key: 'M15 4a5 5 0 1 0 0 10 5 5 0 0 0 0-10zM11.5 12.5 4 20M6.5 17.5 9 20M9 15l2 2M16.5 7.5h.01',
		alert: 'M12 4 2.5 20h19zM12 10v4M12 17h.01',
		play: 'M7.5 5.2v13.6a.8.8 0 0 0 1.2.7l11-6.8a.8.8 0 0 0 0-1.4l-11-6.8a.8.8 0 0 0-1.2.7z',
		pause: 'M6.5 5h4v14h-4zM13.5 5h4v14h-4z',
		previous: 'M6 5v14M19 5.5v13L9 12z',
		next: 'M18 5v14M5 5.5v13L15 12z',
		chevronDown: 'M5 9l7 7 7-7',
		folder:
			'M3.5 7A1.5 1.5 0 0 1 5 5.5h4l2 2h8A1.5 1.5 0 0 1 20.5 9v8.5A1.5 1.5 0 0 1 19 19H5a1.5 1.5 0 0 1-1.5-1.5z',
		file: 'M6.5 3.5h7l4 4v13h-11zM13.5 3.5v4h4',
		image: 'M4 5h16v14H4zM4 16l5-5 4 4 3-3 4 4M15.5 9h.01',
		captions: 'M3.5 5.5h17v13h-17zM7 10.5h4M7 14h10M13.5 10.5H17',
		video: 'M3.5 6.5h12v11h-12zM15.5 10.5l5-3v9l-5-3',
		speaker: 'M4 9.5h3.5L12 6v12l-4.5-3.5H4zM15.5 9a4 4 0 0 1 0 6M18 6.5a7.5 7.5 0 0 1 0 11',
		speakerLow: 'M4 9.5h3.5L12 6v12l-4.5-3.5H4zM15.5 9a4 4 0 0 1 0 6',
		speakerMuted: 'M4 9.5h3.5L12 6v12l-4.5-3.5H4zM15.5 9.5l5 5M20.5 9.5l-5 5',
		moon: 'M19.5 14.5A7.5 7.5 0 0 1 9.5 4.5a7.5 7.5 0 1 0 10 10z',
		list: 'M9 6h11M9 12h11M9 18h11M4.5 6h.01M4.5 12h.01M4.5 18h.01',
		star: 'M12 3.8l2.5 5.1 5.6.8-4 4 1 5.6-5.1-2.7-5.1 2.7 1-5.6-4-4 5.6-.8z',
		trash: 'M4.5 7h15M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3',
		download: 'M12 4v11M7.5 10.5 12 15l4.5-4.5M5 19.5h14',
		fullscreen: 'M4 9V4h5M15 4h5v5M20 15v5h-5M9 20H4v-5',
		fullscreenExit: 'M9 4v5H4M20 9h-5V4M15 20v-5h5M4 15h5v5',
		pip: 'M3.5 5.5h17v13h-17zM12.5 12h6v5h-6z',
		close: 'M6.5 6.5l11 11M17.5 6.5l-11 11',
		expand: 'M14 4h6v6M20 4l-6.5 6.5M10 20H4v-6M4 20l6.5-6.5'
	} as const;

	export type IconName = keyof typeof PATHS;
</script>

<script lang="ts">
	interface Props {
		name: IconName;
		size?: number;
		strokeWidth?: number;
		filled?: boolean;
		class?: string;
	}

	let { name, size = 24, strokeWidth = 1.8, filled = false, class: className = '' }: Props =
		$props();
</script>

<!-- Decorative: the surrounding button or link carries the accessible name. -->
<svg
	class={className}
	width={size}
	height={size}
	viewBox="0 0 24 24"
	fill={filled ? 'currentColor' : 'none'}
	stroke="currentColor"
	stroke-width={strokeWidth}
	stroke-linecap="round"
	stroke-linejoin="round"
	aria-hidden="true"
	focusable="false"
>
	<path d={PATHS[name]} />
</svg>
