<script lang="ts">
	import Rich from './Rich.svelte';
	import { fill, i18n } from '$lib/i18n/index.svelte';
	import { onMount } from 'svelte';
	import { networkProxyGet, networkProxySet, type ProxyStatus } from '$lib/api/security';
	import { friendlyError, describeMessage } from '$lib/errors';

	let status = $state<ProxyStatus | null>(null);
	let draft = $state('');
	let error = $state<string | null>(null);
	let busy = $state(false);

	onMount(async () => {
		try {
			status = await networkProxyGet();
			draft = status.url ?? '';
		} catch (e) {
			error = friendlyError(e);
		}
	});

	async function save(url: string | null) {
		busy = true;
		error = null;
		try {
			status = await networkProxySet(url);
			draft = status.url ?? '';
		} catch (e) {
			error = friendlyError(e);
		} finally {
			busy = false;
		}
	}
</script>

<section class="cu-proxy">
	<h3>{i18n.t.proxy.title}</h3>
	<p class="cu-note">
		<Rich text={i18n.t.proxy.note} />
	</p>
	{#if status?.state === 'broken'}
		<p class="cu-error" role="alert">
			{fill(i18n.t.proxy.invalid, { reason: describeMessage(status.reason ?? '') })}
		</p>
	{/if}
	<form
		class="cu-proxy-form"
		onsubmit={(e) => {
			e.preventDefault();
			save(draft.trim() || null);
		}}
	>
		<label class="cu-field">
			<span>{i18n.t.proxy.address}</span>
			<input
				type="text"
				placeholder="socks5://127.0.0.1:9050"
				autocomplete="off"
				spellcheck="false"
				bind:value={draft}
			/>
		</label>
		<div class="cu-proxy-actions">
			<button type="submit" class="cu-btn primary" disabled={busy}>{i18n.t.common.save}</button>
			<button
				type="button"
				class="cu-btn"
				disabled={busy || status?.state === 'direct'}
				onclick={() => save(null)}
			>
				{i18n.t.proxy.remove}
			</button>
		</div>
	</form>
	<p class="cu-note">
		{#if status?.state === 'active'}
			{fill(i18n.t.proxy.active, { url: status.url ?? '' })}
		{:else if status?.state === 'direct'}
			{i18n.t.proxy.direct}
		{/if}
	</p>
	{#if error}
		<p class="cu-error" role="alert">{error}</p>
	{/if}
</section>

<style>
	.cu-proxy {
		margin-top: 1.5rem;
	}
	.cu-proxy-form {
		display: flex;
		flex-direction: column;
		gap: 0.5rem;
		max-width: 32rem;
	}
	.cu-proxy-actions {
		display: flex;
		gap: 0.5rem;
	}
</style>
