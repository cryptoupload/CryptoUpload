<script lang="ts">
	import { fill, i18n, languages, setLanguage, systemLanguage } from '$lib/i18n/index.svelte';
	import { openTranslateHelp } from '$lib/state/translateHelp.svelte';

	let { compact = false }: { compact?: boolean } = $props();

	const SYSTEM = 'system';
	const id = $props.id();

	async function onchange(event: Event) {
		const value = (event.currentTarget as HTMLSelectElement).value;
		await setLanguage(value === SYSTEM ? null : value);
	}
</script>

<div class="cu-language" class:compact>
	<label class="cu-language-label" for={id}>
		<svg class="cu-i" width="14" height="14" viewBox="0 0 24 24" aria-hidden="true">
			<circle cx="12" cy="12" r="8.5" /><path d="M3.5 12h17M12 3.5c2.5 2.6 2.5 14.4 0 17M12 3.5c-2.5 2.6-2.5 14.4 0 17" />
		</svg>
		<span class:cu-visually-hidden={compact}>{i18n.t.language.label}</span>
	</label>
	<select {id} value={i18n.preference ?? SYSTEM} {onchange}>
		<option value={SYSTEM}>{fill(i18n.t.language.system, { name: systemLanguage().name })}</option>
		{#each languages as l (l.code)}
			<option value={l.code} lang={l.tag} dir={l.dir}>{l.name}</option>
		{/each}
	</select>
	<button type="button" class="cu-text-link cu-language-help" onclick={openTranslateHelp}>
		{i18n.t.language.help}
	</button>
</div>
{#if i18n.language.draft}
	<p class="cu-language-draft">{i18n.t.language.draft}</p>
{/if}

<style>
	.cu-language {
		display: flex;
		align-items: center;
		gap: 12px;
	}

	.cu-language-label {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		flex: 0 0 96px;
		font-size: 13px;
		color: var(--cu-muted);
	}

	.cu-language.compact {
		justify-content: center;
		gap: 6px;
		margin-top: 18px;
	}

	.cu-language.compact .cu-language-label {
		flex: none;
	}

	.cu-language select {
		width: auto;
		max-width: 100%;
	}

	.cu-language-help {
		font-size: 12px;
	}

	.cu-language-draft {
		margin: 8px 0 0;
		font-size: 12px;
		color: var(--cu-muted);
	}

	.cu-visually-hidden {
		position: absolute;
		width: 1px;
		height: 1px;
		overflow: hidden;
		clip-path: inset(50%);
		white-space: nowrap;
	}
</style>
