<script lang="ts">
	import { fill, i18n } from '$lib/i18n/index.svelte';
	import { onMount, tick } from 'svelte';

	let {
		title,
		description,
		confirmLabel,
		typedWord = null,
		busy = false,
		error = null,
		onconfirm,
		oncancel,
	}: {
		title: string;
		description: string;
		confirmLabel: string;
		typedWord?: string | null;
		busy?: boolean;
		error?: string | null;
		onconfirm: () => void;
		oncancel: () => void;
	} = $props();

	let typed = $state('');
	let inputEl = $state<HTMLInputElement | null>(null);
	let dialogEl = $state<HTMLDivElement | null>(null);

	const confirmDisabled = $derived(
		busy || (typedWord !== null && typed.trim().toUpperCase() !== typedWord.toUpperCase()),
	);

	onMount(async () => {
		await tick();
		(inputEl ?? dialogEl?.querySelector<HTMLElement>('button'))?.focus();
	});

	function onKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') {
			e.stopPropagation();
			oncancel();
		} else if (e.key === 'Enter' && typedWord === null && !confirmDisabled) {
			onconfirm();
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

<div
	class="cu-confirm-overlay"
	onkeydown={onKeydown}
	role="presentation"
>
	<div
		class="cu-confirm-dialog"
		role="alertdialog"
		aria-modal="true"
		aria-labelledby="cu-confirm-title"
		aria-describedby="cu-confirm-desc"
		tabindex="-1"
		bind:this={dialogEl}
		onkeydown={trapFocus}
	>
		<h2 id="cu-confirm-title">{title}</h2>
		<p id="cu-confirm-desc">{description}</p>
		{#if typedWord !== null}
			<label class="cu-confirm-label" for="cu-confirm-input">{fill(i18n.t.confirm.typeToConfirm, { word: typedWord })}</label>
			<input
				id="cu-confirm-input"
				class="cu-input cu-mono cu-confirm-input"
				bind:this={inputEl}
				bind:value={typed}
				autocomplete="off"
			/>
		{/if}
		{#if error}
			<p class="cu-confirm-error" role="alert">{error}</p>
		{/if}
		<div class="cu-confirm-btns">
			<button type="button" class="cu-btn" onclick={oncancel}>
				{i18n.t.common.cancel}
			</button>
			<button
				type="button"
				class="cu-btn danger"
				disabled={confirmDisabled}
				onclick={onconfirm}
			>
				{busy ? i18n.t.common.working : confirmLabel}
			</button>
		</div>
	</div>
</div>

<style>
	.cu-confirm-overlay {
		position: fixed;
		inset: 0;
		z-index: 100;
		display: grid;
		place-items: center;
		background: var(--cu-overlay);
		animation: cu-confirm-fade 150ms ease;
	}

	.cu-confirm-dialog {
		width: min(440px, calc(100vw - 32px));
		padding: 26px 28px 22px;
		background: var(--cu-surface);
		border: 1px solid var(--cu-line-strong);
		border-top: 2px solid var(--cu-conflict);
		border-radius: var(--cu-radius-lg);
		color: var(--cu-text);
		animation: cu-confirm-in 220ms var(--cu-ease-out);
	}

	.cu-confirm-dialog h2 {
		margin: 0;
		font-size: 18px;
		font-weight: 600;
		letter-spacing: -0.01em;
	}

	.cu-confirm-dialog p#cu-confirm-desc {
		margin: 8px 0 18px;
		color: var(--cu-muted);
		font-size: 13px;
	}

	.cu-confirm-label {
		display: block;
		font-size: 13px;
		margin-bottom: 6px;
	}

	.cu-confirm-input {
		width: 100%;
		margin-bottom: 20px;
	}

	.cu-confirm-error {
		color: var(--cu-conflict);
		font-size: 13px;
		margin: -6px 0 16px;
	}

	.cu-confirm-btns {
		display: flex;
		justify-content: flex-end;
		gap: 8px;
	}

	@keyframes cu-confirm-fade {
		from {
			opacity: 0;
		}
		to {
			opacity: 1;
		}
	}

	@keyframes cu-confirm-in {
		from {
			opacity: 0;
			transform: translateY(8px) scale(0.985);
		}
		to {
			opacity: 1;
			transform: none;
		}
	}

	:global(:root[data-motion='off']) .cu-confirm-overlay,
	:global(:root[data-motion='off']) .cu-confirm-dialog {
		animation: none;
	}
</style>
