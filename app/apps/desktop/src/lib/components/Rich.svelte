<script lang="ts">
	import { fill } from '$lib/i18n/index.svelte';

	let { text, values = {}, strongClass = '' }: { text: string; values?: Record<string, string | number>; strongClass?: string } = $props();

	type Piece = { tag: '' | 'b' | 'i' | 'code' | 'kbd' | 'br'; text: string };

	function split(source: string): Piece[] {
		const out: Piece[] = [];
		const re = /<(b|i|code|kbd)>(.*?)<\/\1>|<br>/gs;
		let last = 0;
		for (const m of source.matchAll(re)) {
			if (m.index > last) out.push({ tag: '', text: source.slice(last, m.index) });
			out.push(m[1] ? { tag: m[1] as Piece['tag'], text: m[2] } : { tag: 'br', text: '' });
			last = m.index + m[0].length;
		}
		if (last < source.length) out.push({ tag: '', text: source.slice(last) });
		return out;
	}

	const pieces = $derived(split(text).map((p) => ({ tag: p.tag, text: fill(p.text, values) })));
</script>

{#each pieces as p, i (i)}{#if p.tag === 'b'}<strong class={strongClass || undefined}>{p.text}</strong>{:else if p.tag === 'i'}<em>{p.text}</em>{:else if p.tag === 'code'}<code class="cu-mono">{p.text}</code>{:else if p.tag === 'kbd'}<kbd>{p.text}</kbd>{:else if p.tag === 'br'}<br />{:else}{p.text}{/if}{/each}
