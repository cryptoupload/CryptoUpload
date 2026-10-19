<script lang="ts">
	import { i18n } from '$lib/i18n/index.svelte';
	import { friendlyError } from '$lib/errors';
	import { formatBytes } from '$lib/format';
	import { storageUsage, type JobUsage } from '$lib/api/usage';

	let usages = $state<JobUsage[]>([]);
	let loading = $state(true);
	let error = $state<string | null>(null);

	async function load() {
		loading = true;
		error = null;
		try {
			usages = await storageUsage();
		} catch (e) {
			error = friendlyError(e);
		} finally {
			loading = false;
		}
	}

	load();

	const totalBytes = $derived(usages.reduce((sum, u) => sum + u.totalBytes, 0));
	const totalVersionBytes = $derived(usages.reduce((sum, u) => sum + u.versionBytes, 0));
</script>

<div class="cu-page">
	<div class="cu-toolbar">
		<h1 class="cu-page-title">{i18n.t.usage.title}</h1>
		<button type="button" class="cu-btn sm" onclick={load} disabled={loading}>{i18n.t.common.refresh}</button>
	</div>

	<p class="cu-note">
		{i18n.t.usage.note}
	</p>

	{#if error}
		<p class="cu-error" role="alert">{error}</p>
	{/if}

	{#if loading}
		<p class="cu-note">{i18n.t.common.loading}</p>
	{:else if usages.length === 0}
		<p class="cu-note">{i18n.t.usage.empty}</p>
	{:else}
		<div class="cu-totals">
			<div class="cu-total-tile">
				<span class="cu-total-label">{i18n.t.usage.totalBytes}</span>
				<span class="cu-total-value">{formatBytes(totalBytes)}</span>
			</div>
			<div class="cu-total-tile">
				<span class="cu-total-label">{i18n.t.usage.totalVersionBytes}</span>
				<span class="cu-total-value">
					{formatBytes(totalVersionBytes)}
				</span>
			</div>
		</div>

		<table class="cu-usage-table">
			<thead>
				<tr>
					<th>{i18n.t.usage.colJob}</th>
					<th>{i18n.t.usage.colFiles}</th>
					<th>{i18n.t.usage.colSpace}</th>
					<th>{i18n.t.usage.colVersions}</th>
					<th>{i18n.t.usage.colVersionSpace}</th>
					<th>{i18n.t.usage.colTransferred}</th>
				</tr>
			</thead>
			<tbody>
				{#each usages as usage (usage.jobName)}
					<tr>
						<td class="cu-mono">{usage.jobName}</td>
						<td>{usage.fileCount}</td>
						<td>{formatBytes(usage.totalBytes)}</td>
						<td>{usage.versionCount}</td>
						<td>
							{formatBytes(usage.versionBytes)}
						</td>
						<td>
							{formatBytes(usage.totalBytesTransferredLifetime)}
						</td>
					</tr>
				{/each}
			</tbody>
		</table>
	{/if}
</div>

<style>
	.cu-page {
		padding: 30px 40px 26px;
	}

	.cu-toolbar {
		display: flex;
		align-items: center;
		justify-content: space-between;
		margin-bottom: 16px;
	}

	.cu-note {
		color: var(--cu-muted);
		font-size: 13px;
		max-width: 640px;
		margin: 0 0 16px;
	}

	.cu-error {
		background: color-mix(in srgb, var(--cu-conflict) 15%, transparent);
		border: 1px solid var(--cu-conflict);
		color: var(--cu-conflict);
		border-radius: var(--cu-radius-sm);
		padding: 8px 10px;
		font-size: 13px;
		margin: 0 0 16px;
	}

	.cu-totals {
		display: flex;
		gap: 12px;
		margin-bottom: 20px;
	}

	.cu-total-tile {
		background: var(--cu-surface);
		border: 1px solid var(--cu-line);
		border-radius: var(--cu-radius-lg);
		padding: 12px 16px;
		display: flex;
		flex-direction: column;
		gap: 4px;
	}

	.cu-total-label {
		font-size: 12px;
		color: var(--cu-muted);
	}

	.cu-total-value {
		font-size: 18px;
		font-weight: 600;
	}

	.cu-usage-table {
		width: 100%;
		border-collapse: collapse;
		font-size: 13px;
	}

	.cu-usage-table th {
		text-align: start;
		color: var(--cu-muted);
		font-weight: 500;
		font-size: 12px;
		padding: 6px 10px;
		border-bottom: 1px solid var(--cu-line);
	}

	.cu-usage-table td {
		padding: 8px 10px;
		border-bottom: 1px solid var(--cu-line);
	}
</style>
