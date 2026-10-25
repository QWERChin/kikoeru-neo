import type { IconName } from './components/Icon.svelte';

export interface NavItem {
	href: string;
	label: string;
	icon: IconName;
}

export const SIDEBAR_ITEMS: NavItem[] = [
	{ href: '/library', label: '资料库', icon: 'library' },
	{ href: '/recent', label: '最近播放', icon: 'clock' },
	{ href: '/favorites', label: '收藏', icon: 'heart' },
	{ href: '/reviews', label: '我的评价', icon: 'star' }
];

// Phones keep five tabs like iOS. 我的评价 is reached from 设置 there.
export const TAB_ITEMS: NavItem[] = [
	{ href: '/library', label: '资料库', icon: 'library' },
	{ href: '/search', label: '搜索', icon: 'search' },
	{ href: '/favorites', label: '收藏', icon: 'heart' },
	{ href: '/recent', label: '最近', icon: 'clock' },
	{ href: '/settings', label: '设置', icon: 'settings' }
];

export function isActive(pathname: string, href: string): boolean {
	return pathname === href || pathname.startsWith(`${href}/`);
}
