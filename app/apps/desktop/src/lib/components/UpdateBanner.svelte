<script lang="ts">
	import { fill, i18n } from '$lib/i18n/index.svelte';
	import { updateState, installUpdate } from '$lib/state/update.svelte';
</script>

{#if updateState.pending}
	<div class="cu-update-banner" role="status" aria-live="polite">
		<span class="cu-update-label">
			{i18n.t.update.available} <strong>{updateState.pending.version}</strong>
		</span>
		<button
			type="button"
			class="cu-update-install"
			disabled={updateState.installing}
			onclick={installUpdate}
		>
			{updateState.installing ? i18n.t.update.installing : i18n.t.update.install}
		</button>
		<button
			type="button"
			class="cu-update-dismiss"
			aria-label={i18n.t.update.dismiss}
			onclick={() => (updateState.pending = null)}
		>
			<svg class="cu-i"><use href="#cu-i-x" /></svg>
		</button>
	</div>
{/if}

{#if updateState.error}
	<div class="cu-update-error" role="alert">
		{fill(i18n.t.update.error, { error: updateState.error })}
		<button
			type="button"
			onclick={() => (updateState.error = null)}
			aria-label={i18n.t.common.close}
		>
			<svg class="cu-i"><use href="#cu-i-x" /></svg>
		</button>
	</div>
{/if}

<style>
	.cu-update-banner {
		position: fixed;
		bottom: 16px;
		inset-inline-end: 16px;
		display: flex;
		align-items: center;
		gap: 10px;
		background: var(--cu-seal);
		color: var(--cu-on-seal);
		border-radius: var(--cu-radius-lg);
		padding: 10px 14px;
		font-size: 13px;
		box-shadow: var(--cu-shadow);
		z-index: 9999;
		max-width: 420px;
	}

	.cu-update-label {
		flex: 1;
	}

	.cu-update-install {
		background: color-mix(in srgb, var(--cu-on-seal) 12%, transparent);
		border: 1px solid var(--cu-on-seal);
		color: var(--cu-on-seal);
		border-radius: var(--cu-radius-sm);
		height: 28px;
		padding: 0 12px;
		font-size: 12px;
		cursor: pointer;
		white-space: nowrap;
	}

	.cu-update-install:disabled {
		opacity: 0.6;
		cursor: default;
	}

	.cu-update-dismiss {
		display: inline-flex;
		align-items: center;
		background: none;
		border: none;
		color: var(--cu-on-seal);
		border-radius: var(--cu-radius-sm);
		cursor: pointer;
		padding: 4px;
		line-height: 1;
	}

	.cu-update-error {
		position: fixed;
		bottom: 16px;
		inset-inline-end: 16px;
		display: flex;
		align-items: center;
		gap: 8px;
		background: var(--cu-conflict-strong);
		color: var(--cu-on-seal);
		border-radius: var(--cu-radius-sm);
		padding: 10px 14px;
		font-size: 12px;
		z-index: 9999;
		max-width: 420px;
	}

	.cu-update-error button {
		display: inline-flex;
		align-items: center;
		background: none;
		border: none;
		color: var(--cu-on-seal);
		border-radius: var(--cu-radius-sm);
		padding: 4px;
		opacity: 0.75;
		cursor: pointer;
	}
</style>
