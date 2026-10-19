<script lang="ts">
	import { fill, i18n } from '$lib/i18n/index.svelte';
	import { onMount, tick } from 'svelte';

	type ClashChoice = 'both' | 'replace' | 'skip';

	let {
		name,
		isDir,
		canReplace,
		remaining,
		onchoose,
	}: {
		name: string;
		isDir: boolean;
		canReplace: boolean;
		remaining: number;
		onchoose: (choice: ClashChoice, applyToAll: boolean) => void;
	} = $props();

	let applyToAll = $state(false);
	let dialogEl = $state<HTMLDivElement | null>(null);
	let bothBtn = $state<HTMLButtonElement | null>(null);

	onMount(async () => {
		await tick();
		bothBtn?.focus();
	});

	function onKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') {
			e.stopPropagation();
			onchoose('skip', false);
		}
	}

	function trapFocus(e: KeyboardEvent) {
		if (e.key !== 'Tab' || !dialogEl) return;
		const focusable = [...dialogEl.querySelectorAll<HTMLElement>('button, input')].filter(
			(el) => !el.hasAttribute('disabled'),
		);
		if (focusable.length === 0) return;
		const first = focusable[0];
		const last = focusable[focusable.length - 1];
		if (e.shiftKey && document.activeElement === first) {
			e.preventDefault();
			last.focus();
		} else if (!e.shiftKey && document.activeElement === last) {
			e.preventDefault();
			first.focus();
		}
	}
</script>

<div class="cu-clash-overlay" onkeydown={onKeydown} role="presentation">
	<div
		class="cu-clash-dialog"
		role="alertdialog"
		aria-modal="true"
		aria-labelledby="cu-clash-title"
		aria-describedby="cu-clash-desc"
		tabindex="-1"
		bind:this={dialogEl}
		onkeydown={trapFocus}
	>
		<h2 id="cu-clash-title">{fill(i18n.t.clash.title, { name })}</h2>
		<p id="cu-clash-desc">
			{#if isDir}
				{i18n.t.clash.folderBody}
			{:else}
				{i18n.t.clash.fileBody}
			{/if}
		</p>
		{#if remaining > 0}
			<label class="cu-clash-all">
				<input type="checkbox" bind:checked={applyToAll} />
				{fill(i18n.t.clash.applyToAll, { remaining })}
			</label>
		{/if}
		<div class="cu-clash-btns">
			<button type="button" class="cu-btn" onclick={() => onchoose('skip', applyToAll)}>
				{i18n.t.clash.skip}
			</button>
			{#if canReplace}
				<button
					type="button"
					class="cu-btn danger"
					onclick={() => onchoose('replace', applyToAll)}
				>
					{i18n.t.clash.replace}
				</button>
			{/if}
			<button
				type="button"
				class="cu-btn primary"
				bind:this={bothBtn}
				onclick={() => onchoose('both', applyToAll)}
			>
				{i18n.t.clash.keepBoth}
			</button>
		</div>
	</div>
</div>

<style>
	.cu-clash-overlay {
		position: fixed;
		inset: 0;
		z-index: 100;
		display: grid;
		place-items: center;
		background: var(--cu-overlay);
	}

	.cu-clash-dialog {
		width: min(460px, calc(100vw - 32px));
		padding: 26px 28px 22px;
		background: var(--cu-surface);
		border: 1px solid var(--cu-line-strong);
		border-radius: var(--cu-radius-lg);
		color: var(--cu-text);
	}

	.cu-clash-dialog h2 {
		margin: 0;
		font-size: 18px;
		font-weight: 600;
		letter-spacing: -0.01em;
		overflow-wrap: anywhere;
	}

	.cu-clash-dialog p {
		margin: 8px 0 16px;
		color: var(--cu-muted);
		font-size: 13px;
	}

	.cu-clash-all {
		display: flex;
		align-items: center;
		gap: 8px;
		font-size: 13px;
		margin-bottom: 18px;
	}

	.cu-clash-btns {
		display: flex;
		justify-content: flex-end;
		flex-wrap: wrap;
		gap: 8px;
	}
</style>
