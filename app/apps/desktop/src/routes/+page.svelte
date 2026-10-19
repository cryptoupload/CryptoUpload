<script lang="ts">
	import { i18n } from '$lib/i18n/index.svelte';
	import UnlockScreen from '$lib/components/UnlockScreen.svelte';
	import RecoveryReveal from '$lib/components/RecoveryReveal.svelte';
	import Wizard from '$lib/components/Wizard.svelte';
	import AppShell from '$lib/components/AppShell.svelte';
	import VaultSummary from '$lib/components/VaultSummary.svelte';
	import Browse from '$lib/components/Browse.svelte';
	import Jobs from '$lib/components/Jobs.svelte';
	import Usage from '$lib/components/Usage.svelte';
	import RemotesManager from '$lib/components/RemotesManager.svelte';
	import Settings from '$lib/components/Settings.svelte';
	import DepositScreen from '$lib/components/DepositScreen.svelte';
	import OpenShareScreen from '$lib/components/OpenShareScreen.svelte';
	import SyncStrip from '$lib/components/SyncStrip.svelte';
	import OpeningSequence from '$lib/components/OpeningSequence.svelte';
	import { session } from '$lib/state/session.svelte';
</script>

{#if session.screen === 'wizard-intro' || session.screen === 'wizard-choice' || session.screen === 'wizard-setup' || session.screen === 'wizard-shortcuts'}
	<Wizard />
{:else if session.screen === 'unlock'}
	<UnlockScreen />
{:else if session.screen === 'recovery-reveal'}
	<RecoveryReveal />
{:else if session.screen === 'remotes'}
	<div class="cu-standalone-remotes">
		<button
			type="button"
			class="cu-back cu-btn ghost sm"
			onclick={() => (session.screen = session.screenBeforeRemotes)}
		>
			{i18n.t.common.back}
		</button>
		<RemotesManager />
	</div>
{:else if session.screen === 'deposit'}
	<div class="cu-standalone-remotes">
		<DepositScreen />
	</div>
{:else if session.screen === 'open-share'}
	<div class="cu-standalone-remotes">
		<OpenShareScreen />
	</div>
{:else if session.screen !== 'loading'}
	<AppShell>
		{#snippet strip()}
			<SyncStrip />
		{/snippet}
		{#if session.section === 'browse'}
			<Browse />
		{:else if session.section === 'jobs'}
			<Jobs />
		{:else if session.section === 'usage'}
			<Usage />
		{:else if session.section === 'remotes'}
			<RemotesManager />
		{:else if session.section === 'settings'}
			<Settings />
		{:else}
			<VaultSummary entering={session.opening} />
		{/if}
	</AppShell>
	{#if session.opening}
		<OpeningSequence ondone={() => (session.opening = false)} />
	{/if}
{/if}

<style>
	.cu-standalone-remotes {
		height: 100%;
		overflow: auto;
	}

	.cu-back {
		margin: 16px 0 0 28px;
	}
</style>
