<script lang="ts">
	import Rich from './Rich.svelte';
	import { i18n } from '$lib/i18n/index.svelte';
	let {
		checked = $bindable(false),
		decoys = $bindable(false),
	}: {
		checked?: boolean;
		decoys?: boolean;
	} = $props();

	$effect(() => {
		if (!checked) decoys = false;
	});
</script>

<div class="cu-private-profile">
	<label class="cu-private-toggle">
		<input type="checkbox" bind:checked />
		<span>{i18n.t.privateProfile.toggle}</span>
	</label>
	{#if checked}
		<ul class="cu-private-notes">
			<li><Rich text={i18n.t.privateProfile.hides} /></li>
			<li><Rich text={i18n.t.privateProfile.doesNotHide} /></li>
			<li><Rich text={i18n.t.privateProfile.costs} /></li>
			<li><Rich text={i18n.t.privateProfile.noChange} /></li>
		</ul>
		<label class="cu-private-toggle cu-private-sub">
			<input type="checkbox" bind:checked={decoys} />
			<span>{i18n.t.privateProfile.decoysToggle}</span>
		</label>
		{#if decoys}
			<ul class="cu-private-notes">
				<li><Rich text={i18n.t.privateProfile.decoysHides} /></li>
				<li><Rich text={i18n.t.privateProfile.decoysDoesNotHide} /></li>
				<li><Rich text={i18n.t.privateProfile.decoysCosts} /></li>
			</ul>
		{/if}
	{/if}
</div>

<style>
	.cu-private-profile {
		margin: 0.5rem 0;
		text-align: start;
	}
	.cu-private-toggle {
		display: flex;
		align-items: center;
		gap: 0.5rem;
		cursor: pointer;
	}
	.cu-private-sub {
		margin-top: 0.6rem;
	}
	.cu-private-notes {
		margin: 0.5rem 0 0;
		padding-inline-start: 1.2rem;
		font-size: 0.85rem;
		opacity: 0.85;
	}
	.cu-private-notes li {
		margin: 0.2rem 0;
	}
</style>
