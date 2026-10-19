<script lang="ts">
	import { fill, i18n, plural } from '$lib/i18n/index.svelte';
	import { friendlyError } from '$lib/errors';
	import { onMount } from 'svelte';
	import { vaultGenerateRecoveryPhrase } from '$lib/api/vault';
	import { session } from '$lib/state/session.svelte';
	import { checkAnswers, pickChallengeIndices } from '$lib/recoveryCheck';
	import { recoveryKitState, generateRecoveryKit } from '$lib/state/recoveryKit.svelte';

	type Step = 'reveal' | 'verify' | 'offer-kit';

	let words = $state<string[] | null>(null);
	let error = $state<string | null>(null);
	let step = $state<Step>('reveal');
	let challengeIndices = $state<number[]>([]);
	let answers = $state<string[]>([]);
	let verifyError = $state<string | null>(null);
	let skipConfirmOpen = $state(false);

	onMount(async () => {
		try {
			words = await vaultGenerateRecoveryPhrase(false);
			if (session.vaultInfo) {
				session.vaultInfo = { ...session.vaultInfo, hasRecoveryPhrase: true };
			}
		} catch (e) {
			error = friendlyError(e);
		}
	});

	function goToVerify() {
		if (!words) return;
		challengeIndices = pickChallengeIndices(3, words.length);
		answers = challengeIndices.map(() => '');
		verifyError = null;
		skipConfirmOpen = false;
		step = 'verify';
	}

	function submitVerify() {
		if (!words) return;
		if (checkAnswers(words, challengeIndices, answers)) {
			session.recoveryPhraseConfirmed = true;
			step = 'offer-kit';
			return;
		}
		verifyError = i18n.t.recovery.mismatch;
		step = 'reveal';
	}

	let kitPanelWasOpen = false;
	$effect(() => {
		if (recoveryKitState.html) {
			kitPanelWasOpen = true;
		} else if (kitPanelWasOpen && step === 'offer-kit') {
			kitPanelWasOpen = false;
			continueToApp();
		}
	});

	function continueToApp() {
		session.screen = session.afterRecoveryScreen;
		session.afterRecoveryScreen = 'unlocked';
	}
</script>

<div class="cu-gate">
	<div class="cu-card">
		{#if step === 'reveal'}
			<h1>{i18n.t.recovery.title}</h1>
			<p class="cu-subtitle">
				{i18n.t.recovery.intro}
			</p>

			{#if error}
				<p class="cu-error" role="alert">{error}</p>
			{:else if !words}
				<p class="cu-subtitle">{i18n.t.recovery.generating}</p>
			{:else}
				{#if verifyError}
					<p class="cu-error" role="alert">{verifyError}</p>
				{/if}
				<ol class="cu-words cu-mono">
					{#each words as word, i (i)}
						<li>
							<span class="cu-idx">{i + 1}</span><span
								>{word}</span
							>
						</li>
					{/each}
				</ol>

				<button class="cu-primary" onclick={goToVerify}>
					{i18n.t.recovery.written}
				</button>
			{/if}
		{:else if step === 'verify'}
			<h1>{i18n.t.recovery.confirmTitle}</h1>
			<p class="cu-subtitle">
				{plural(challengeIndices.length, i18n.t.recovery.confirmIntro)}
			</p>

			<form
				onsubmit={(e) => {
					e.preventDefault();
					submitVerify();
				}}
			>
				{#each challengeIndices as idx, i (idx)}
					<label class="cu-field">
						<span>{fill(i18n.t.recovery.wordNumber, { n: idx + 1 })}</span>
						<input
							class="cu-mono"
							type="text"
							autocomplete="off"
							spellcheck="false"
							bind:value={answers[i]}
						/>
					</label>
				{/each}

				<button class="cu-primary" type="submit">
					{i18n.t.common.confirm}
				</button>
			</form>

			<button
				type="button"
				class="cu-skip-link cu-btn ghost sm block"
				onclick={() => (skipConfirmOpen = true)}
			>
				{i18n.t.common.notNow}
			</button>

			{#if skipConfirmOpen}
				<div class="cu-skip-confirm">
					<p>
						{i18n.t.recovery.skipWarning}
					</p>
					<div class="cu-skip-actions">
						<button
							type="button"
							class="cu-btn"
							onclick={() => (skipConfirmOpen = false)}
						>
							{i18n.t.common.goBack}
						</button>
						<button
							type="button"
							class="cu-btn danger"
							onclick={continueToApp}
						>
							{i18n.t.recovery.skipAnyway}
						</button>
					</div>
				</div>
			{/if}
		{:else}
			<h1>{i18n.t.recovery.confirmed}</h1>
			<p class="cu-subtitle">
				{i18n.t.recovery.kitOffer}
			</p>

			{#if recoveryKitState.error}
				<p class="cu-error" role="alert">{recoveryKitState.error}</p>
			{/if}

			<button
				class="cu-primary"
				disabled={recoveryKitState.busy}
				onclick={generateRecoveryKit}
			>
				{recoveryKitState.busy ? i18n.t.recovery.kitGenerating : i18n.t.recovery.kitGenerate}
			</button>
			<button
				type="button"
				class="cu-skip-link cu-btn ghost sm block"
				onclick={continueToApp}
			>
				{i18n.t.recovery.kitLater}
			</button>
		{/if}
	</div>
</div>

<style>
	.cu-gate {
		height: 100%;
		display: flex;
		align-items: center;
		justify-content: center;
		padding: 32px;
	}

	.cu-card {
		width: 100%;
		max-width: 460px;
		background: var(--cu-surface);
		border: 1px solid var(--cu-line);
		border-radius: var(--cu-radius-lg);
		box-shadow: var(--cu-shadow);
		padding: 32px 28px;
		animation: cu-card-in var(--cu-motion-slow) var(--cu-ease-out) both;
	}

	h1 {
		margin: 0 0 4px;
		font-family: var(--cu-font-display);
		font-weight: 400;
		font-size: 26px;
		letter-spacing: -0.005em;
	}

	.cu-subtitle {
		margin: 0 0 20px;
		color: var(--cu-muted);
		font-size: 13px;
		line-height: 1.5;
	}

	.cu-words {
		list-style: none;
		margin: 0 0 20px;
		padding: 16px;
		background: var(--cu-bg);
		border: 1px solid var(--cu-line);
		border-radius: var(--cu-radius-lg);
		display: grid;
		grid-template-columns: repeat(3, 1fr);
		gap: 8px 16px;
	}

	.cu-words li {
		font-size: 13px;
		display: flex;
		gap: 6px;
	}

	.cu-idx {
		color: var(--cu-muted);
		min-width: 1.4em;
		text-align: end;
	}

	.cu-field {
		display: block;
		margin: 0 0 14px;
	}

	.cu-field span {
		display: block;
		font-size: 12px;
		color: var(--cu-muted);
		margin-bottom: 4px;
	}

	.cu-field input {
		width: 100%;
		height: 34px;
		background: var(--cu-bg);
		border: 1px solid var(--cu-line-strong);
		border-radius: var(--cu-radius-sm);
		color: var(--cu-text);
		padding: 0 12px;
		font-size: 14px;
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

	.cu-skip-link.cu-btn {
		margin-top: 12px;
		height: auto;
		min-height: 28px;
		padding-block: 5px;
		white-space: normal;
		text-align: center;
	}

	.cu-skip-confirm {
		margin-top: 16px;
		padding: 14px;
		background: color-mix(in srgb, var(--cu-conflict) 12%, transparent);
		border: 1px solid var(--cu-conflict);
		border-radius: var(--cu-radius-lg);
		font-size: 13px;
		line-height: 1.5;
	}

	.cu-skip-confirm p {
		margin: 0 0 12px;
	}

	.cu-skip-actions {
		display: flex;
		gap: 8px;
	}

	.cu-skip-actions button {
		flex: 1;
	}
</style>
