<script lang="ts">
	import { i18n, plural } from '$lib/i18n/index.svelte';
	import { onMount } from 'svelte';
	import { listen } from '@tauri-apps/api/event';
	import { ACCESS_REQUEST_EVENT, driveAccessAnswer, type AccessRequest } from '$lib/api/security';

	type Pending = AccessRequest & { expiresAt: number };
	const ANSWER_WINDOW_MS = 30_000;
	let queue = $state<Pending[]>([]);
	const current = $derived(queue[0] ?? null);

	onMount(() => {
		const stop = listen<AccessRequest>(ACCESS_REQUEST_EVENT, (event) => {
			queue = [...queue, { ...event.payload, expiresAt: Date.now() + ANSWER_WINDOW_MS }];
		});
		const prune = setInterval(() => {
			const now = Date.now();
			if (queue.some((request) => request.expiresAt <= now)) {
				queue = queue.filter((request) => request.expiresAt > now);
			}
		}, 1000);
		return () => {
			clearInterval(prune);
			void stop.then((unlisten) => unlisten());
		};
	});

	async function answer(allow: boolean) {
		const request = current;
		if (!request) return;
		queue = queue.slice(1);
		try {
			await driveAccessAnswer(request.id, allow);
		} catch {
		}
	}
</script>

{#if current}
	<div class="cu-access-backdrop">
		<div class="cu-access" role="alertdialog" aria-modal="true" aria-labelledby="cu-access-title">
			<h2 id="cu-access-title">{i18n.t.driveAccess.title}</h2>
			<p class="cu-mono cu-access-program">{current.program}</p>
			<p>
				{i18n.t.driveAccess.body}
			</p>
			<div class="cu-access-actions">
				<button type="button" class="cu-btn" onclick={() => answer(false)}>
					{i18n.t.driveAccess.deny}
				</button>
				<button type="button" class="cu-btn primary" onclick={() => answer(true)}>
					{i18n.t.driveAccess.allow}
				</button>
			</div>
			{#if queue.length > 1}
				<p class="cu-access-more">{plural(queue.length - 1, i18n.t.driveAccess.moreWaiting)}</p>
			{/if}
		</div>
	</div>
{/if}

<style>
	.cu-access-backdrop {
		position: fixed;
		inset: 0;
		z-index: 60;
		display: grid;
		place-items: center;
		background: rgb(0 0 0 / 55%);
	}

	.cu-access {
		width: min(520px, calc(100vw - 48px));
		padding: 20px 22px;
		background: var(--cu-surface);
		border: 1px solid var(--cu-line-strong);
		border-radius: var(--cu-radius-sm);
		font-size: 13px;
	}

	.cu-access h2 {
		margin: 0 0 12px;
		font-size: 15px;
	}

	.cu-access-program {
		margin: 0 0 12px;
		padding: 8px 10px;
		border: 1px solid var(--cu-line);
		border-radius: var(--cu-radius-sm);
		overflow-wrap: anywhere;
	}

	.cu-access-actions {
		display: flex;
		justify-content: flex-end;
		gap: 8px;
		margin-top: 16px;
	}

	.cu-access-more {
		margin: 10px 0 0;
		color: var(--cu-muted);
	}
</style>
