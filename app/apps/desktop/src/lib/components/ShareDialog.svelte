<script lang="ts">
	import Rich from './Rich.svelte';
	import { fill, i18n } from '$lib/i18n/index.svelte';
	import { onMount, tick } from 'svelte';
	import { save } from '@tauri-apps/plugin-dialog';
	import { passwordStrength } from '$lib/passwordStrength';

	let {
		fileName,
		defaultOut,
		busy,
		error,
		mustConfirmOverwrite,
		oncreate,
		oncancel,
	}: {
		fileName: string;
		defaultOut: string;
		busy: boolean;
		error: string | null;
		mustConfirmOverwrite: boolean;
		oncreate: (args: { password: string; out: string; overwrite: boolean }) => void;
		oncancel: () => void;
	} = $props();

	let password = $state('');
	let confirm = $state('');
	let out = $state('');
	let overwrite = $state(false);
	let dialogEl = $state<HTMLDivElement | null>(null);
	let passwordEl = $state<HTMLInputElement | null>(null);

	const strength = $derived(passwordStrength(password));
	const problem = $derived.by(() => {
		if (!out.trim()) return i18n.t.share.chooseDest;
		if (!password) return i18n.t.share.passwordNeeded;
		if (strength.score === 0) return i18n.t.share.passwordWeak;
		if (password !== confirm) return i18n.t.vaultForm.passwordsDiffer;
		if (mustConfirmOverwrite && !overwrite) return i18n.t.share.exists;
		return null;
	});

	onMount(async () => {
		out = defaultOut;
		await tick();
		passwordEl?.focus();
	});

	async function choose() {
		const picked = await save({
			defaultPath: out || defaultOut,
			filters: [{ name: i18n.t.share.fileTypeName, extensions: ['cushare'] }],
		});
		if (typeof picked === 'string') {
			out = picked;
			overwrite = true;
		}
	}

	function submit(e: Event) {
		e.preventDefault();
		if (problem || busy) return;
		oncreate({ password, out: out.trim(), overwrite });
	}

	function onKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape' && !busy) {
			e.stopPropagation();
			oncancel();
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

<div class="cu-share-overlay" onkeydown={onKeydown} role="presentation">
	<div
		class="cu-share-dialog"
		role="dialog"
		aria-modal="true"
		aria-labelledby="cu-share-title"
		tabindex="-1"
		bind:this={dialogEl}
		onkeydown={trapFocus}
	>
		<h2 id="cu-share-title">{fill(i18n.t.share.title, { name: fileName })}</h2>
		<p class="cu-share-desc">
			<Rich text={i18n.t.share.intro} />
		</p>
		<form onsubmit={submit}>
			<label class="cu-field">
				<span>{i18n.t.share.packagePassword}</span>
				<input
					type="password"
					autocomplete="new-password"
					bind:value={password}
					bind:this={passwordEl}
				/>
			</label>
			{#if password}
				<p class="cu-strength" data-score={strength.score}>
					{fill(i18n.t.vaultForm.strength, { label: strength.label })}
				</p>
			{/if}
			<label class="cu-field">
				<span>{i18n.t.vaultForm.repeatPassword}</span>
				<input
					type="password"
					autocomplete="new-password"
					bind:value={confirm}
				/>
			</label>
			<label class="cu-field">
				<span>{i18n.t.share.destLabel}</span>
				<span class="cu-share-path">
					<input type="text" bind:value={out} />
					<button type="button" class="cu-btn" onclick={choose}>{i18n.t.common.choose}</button>
				</span>
			</label>
			{#if mustConfirmOverwrite}
				<label class="cu-share-overwrite">
					<input type="checkbox" bind:checked={overwrite} />
					{i18n.t.share.overwrite}
				</label>
			{/if}
			{#if error}
				<p class="cu-share-error" role="alert">{error}</p>
			{:else if problem && (password || confirm)}
				<p class="cu-share-hint">{problem}</p>
			{/if}
			<div class="cu-share-btns">
				<button type="button" class="cu-btn" disabled={busy} onclick={oncancel}>
					{i18n.t.common.cancel}
				</button>
				<button type="submit" class="cu-btn primary" disabled={!!problem || busy}>
					{busy ? i18n.t.share.creating : i18n.t.share.submit}
				</button>
			</div>
		</form>
	</div>
</div>

<style>
	.cu-share-overlay {
		position: fixed;
		inset: 0;
		z-index: 100;
		display: grid;
		place-items: center;
		background: var(--cu-overlay);
	}

	.cu-share-dialog {
		width: min(480px, calc(100vw - 32px));
		max-height: calc(100vh - 32px);
		overflow-y: auto;
		padding: 26px 28px 22px;
		background: var(--cu-surface);
		border: 1px solid var(--cu-line-strong);
		border-radius: var(--cu-radius-lg);
		color: var(--cu-text);
	}

	h2 {
		margin: 0;
		font-size: 18px;
		font-weight: 600;
		letter-spacing: -0.01em;
		overflow-wrap: anywhere;
	}

	.cu-share-desc {
		margin: 8px 0 16px;
		color: var(--cu-muted);
		font-size: 13px;
	}

	.cu-field {
		display: flex;
		flex-direction: column;
		gap: 6px;
		font-size: 13px;
		margin-bottom: 12px;
	}

	.cu-field input[type='password'],
	.cu-field input[type='text'] {
		height: 34px;
		background: var(--cu-bg);
		border: 1px solid var(--cu-line-strong);
		color: var(--cu-text);
		border-radius: var(--cu-radius-sm);
		padding: 0 12px;
		font-size: 13px;
		min-width: 0;
	}

	.cu-share-path {
		display: flex;
		gap: 8px;
	}

	.cu-share-path input {
		flex: 1;
	}

	.cu-strength {
		margin: -6px 0 10px;
		font-size: 12px;
		color: var(--cu-verified);
	}

	.cu-strength[data-score='0'] {
		color: var(--cu-conflict);
	}

	.cu-share-overwrite {
		display: flex;
		align-items: center;
		gap: 8px;
		font-size: 13px;
		margin-bottom: 12px;
	}

	.cu-share-hint {
		margin: 0 0 12px;
		font-size: 12px;
		color: var(--cu-muted);
	}

	.cu-share-error {
		margin: 0 0 12px;
		padding: 8px 10px;
		font-size: 13px;
		color: var(--cu-conflict);
		background: color-mix(in srgb, var(--cu-conflict) 15%, transparent);
		border: 1px solid var(--cu-conflict);
		border-radius: var(--cu-radius-sm);
		overflow-wrap: anywhere;
	}

	.cu-share-btns {
		display: flex;
		justify-content: flex-end;
		gap: 8px;
		margin-top: 8px;
	}
</style>
