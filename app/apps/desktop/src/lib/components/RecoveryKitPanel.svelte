<script lang="ts">
	import { fill, i18n } from '$lib/i18n/index.svelte';
	import { recoveryKitState, closeRecoveryKitPanel } from '$lib/state/recoveryKit.svelte';
	import { recoveryKitSave } from '$lib/api/recoveryKit';
	import { open as openDialog } from '@tauri-apps/plugin-dialog';
	import { join } from '@tauri-apps/api/path';
	import { friendlyError } from '$lib/errors';

	let iframeEl = $state<HTMLIFrameElement | undefined>(undefined);
	let saveMessage = $state<string | null>(null);
	let saveError = $state<string | null>(null);

	$effect(() => {
		const html = recoveryKitState.html;
		const frame = iframeEl;
		if (!html || !frame) return;
		const doc = frame.contentDocument;
		if (!doc) return;
		doc.open();
		doc.write(html);
		doc.close();
	});

	function printKit() {
		iframeEl?.contentWindow?.print();
	}

	async function saveKit() {
		if (!recoveryKitState.html) return;
		saveError = null;
		saveMessage = null;
		try {
			const destDir = await openDialog({ directory: true, multiple: false });
			if (typeof destDir !== 'string') return;
			const today = new Date().toISOString().slice(0, 10);
			const destPath = await join(destDir, `cryptoupload-recovery-kit-${today}.html`);
			await recoveryKitSave(destPath);
			saveMessage = fill(i18n.t.recoveryKit.savedTo, { path: destPath });
		} catch (e) {
			saveError = friendlyError(e);
		}
	}

	function close() {
		saveMessage = null;
		saveError = null;
		closeRecoveryKitPanel();
	}
</script>

{#if recoveryKitState.html}
	<div class="cu-kit-overlay">
		<div class="cu-kit-modal">
			<div class="cu-kit-toolbar">
				<p class="cu-kit-title">{i18n.t.recoveryKit.title}</p>
				<div class="cu-kit-actions">
					<button
						type="button"
						class="cu-btn primary"
						onclick={printKit}
					>
						{i18n.t.recoveryKit.print}
					</button>
					<button
						type="button"
						class="cu-btn"
						onclick={saveKit}
					>
						{i18n.t.recoveryKit.saveAsFile}
					</button>
					<button
						type="button"
						class="cu-btn"
						onclick={close}
					>
						{i18n.t.common.close}
					</button>
				</div>
			</div>
			{#if saveMessage}
				<p class="cu-kit-note">{saveMessage}</p>
			{/if}
			{#if saveError}
				<p class="cu-kit-error">{saveError}</p>
			{/if}
			<iframe
				title={i18n.t.recoveryKit.title}
				bind:this={iframeEl}
				class="cu-kit-frame"
			></iframe>
		</div>
	</div>
{/if}

<style>
	.cu-kit-overlay {
		position: fixed;
		inset: 0;
		background: var(--cu-overlay);
		display: flex;
		align-items: center;
		justify-content: center;
		z-index: 200;
		padding: 24px;
	}

	.cu-kit-modal {
		background: var(--cu-surface);
		border: 1px solid var(--cu-line);
		border-radius: var(--cu-radius-lg);
		box-shadow: var(--cu-shadow);
		width: 100%;
		max-width: 820px;
		height: 100%;
		max-height: 90vh;
		display: flex;
		flex-direction: column;
		overflow: hidden;
	}

	.cu-kit-toolbar {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 12px;
		padding: 14px 18px;
		border-bottom: 1px solid var(--cu-line);
	}

	.cu-kit-title {
		margin: 0;
		font-size: 14px;
		font-weight: 600;
	}

	.cu-kit-actions {
		display: flex;
		gap: 8px;
	}

	.cu-kit-note {
		margin: 0;
		padding: 8px 18px 0;
		font-size: 12px;
		color: var(--cu-muted);
	}

	.cu-kit-error {
		margin: 0;
		padding: 8px 18px 0;
		font-size: 12px;
		color: var(--cu-conflict);
	}

	.cu-kit-frame {
		flex: 1;
		border: none;
		width: 100%;
		background: var(--cu-on-seal);
	}
</style>
