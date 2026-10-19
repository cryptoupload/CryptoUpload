<script lang="ts">
	import { openUrl } from '@tauri-apps/plugin-opener';
	import { tick } from 'svelte';
	import { fill, i18n } from '$lib/i18n/index.svelte';
	import { bugReportInfo } from '$lib/api/settings';
	import {
		FEEDBACK_EMAIL,
		REPORT_TEXT_MAX,
		issueUrl,
		mailtoUrl,
		factsLine,
		reportText,
		type Report,
		type ReportKind,
	} from '$lib/bugReport';
	import { closeReportBug, reportBug } from '$lib/state/reportBug.svelte';

	let kind = $state<ReportKind>('bug');
	let text = $state('');
	let withFacts = $state(true);
	let facts = $state.raw<{ version: string; os: string; arch: string } | null>(null);
	let status = $state<string | null>(null);
	let failure = $state<string | null>(null);
	let textEl = $state<HTMLTextAreaElement | null>(null);

	const report = $derived<Report>({
		kind,
		text,
		facts: withFacts && facts ? { ...facts, language: i18n.language.code } : null,
	});
	const empty = $derived(text.trim() === '');

	$effect(() => {
		if (!reportBug.open) return;
		status = null;
		failure = null;
		if (!facts) bugReportInfo().then((f) => (facts = f)).catch(() => (facts = null));
		tick().then(() => textEl?.focus());
	});

	function ready(): boolean {
		status = null;
		failure = empty ? i18n.t.report.empty : null;
		return !empty;
	}

	async function open(url: string, failed: string) {
		try {
			await openUrl(url);
		} catch {
			failure = failed;
		}
	}

	async function writeEmail() {
		if (ready()) await open(mailtoUrl(report), fill(i18n.t.report.mailFailed, { email: FEEDBACK_EMAIL }));
	}

	async function openIssue() {
		if (ready()) await open(issueUrl(report), fill(i18n.t.report.linkFailed, { email: FEEDBACK_EMAIL }));
	}

	async function copy() {
		if (!ready()) return;
		try {
			await navigator.clipboard.writeText(reportText(report));
			status = i18n.t.report.copied;
		} catch {
			failure = i18n.t.report.copyFailed;
		}
	}

	function close() {
		closeReportBug();
	}

	function onKeydown(e: KeyboardEvent) {
		if (e.key === 'Escape') {
			e.stopPropagation();
			close();
		}
	}
</script>

{#if reportBug.open}
	<!-- svelte-ignore a11y_no_static_element_interactions -->
	<div class="cu-rb-overlay" onkeydown={onKeydown}>
		<div class="cu-rb-modal" role="dialog" aria-modal="true" aria-labelledby="cu-rb-title">
			<div class="cu-rb-head">
				<h2 id="cu-rb-title">{i18n.t.report.title}</h2>
				<button type="button" class="cu-btn" onclick={close}>{i18n.t.common.close}</button>
			</div>
			<div class="cu-rb-body">
				<p class="cu-note">{i18n.t.report.lead}</p>

				<div class="cu-rb-kinds" role="radiogroup" aria-label={i18n.t.report.kindLabel}>
					<label class="cu-rb-kind" class:on={kind === 'bug'}>
						<input type="radio" name="cu-rb-kind" value="bug" bind:group={kind} />
						<span>{i18n.t.report.kindBug}</span>
					</label>
					<label class="cu-rb-kind" class:on={kind === 'idea'}>
						<input type="radio" name="cu-rb-kind" value="idea" bind:group={kind} />
						<span>{i18n.t.report.kindIdea}</span>
					</label>
				</div>

				<label class="cu-field">
					<span>{i18n.t.report.text}</span>
					<textarea
						rows="6"
						maxlength={REPORT_TEXT_MAX}
						placeholder={kind === 'bug' ? i18n.t.report.placeholderBug : i18n.t.report.placeholderIdea}
						bind:this={textEl}
						bind:value={text}
						oninput={() => (failure = null)}
					></textarea>
				</label>

				<label class="cu-rb-check">
					<input type="checkbox" bind:checked={withFacts} />
					<span>{i18n.t.report.facts}</span>
				</label>
				<p class="cu-note cu-rb-preview">
					{report.facts ? factsLine(report.facts) : i18n.t.report.noFacts}
				</p>

				<p class="cu-note">{i18n.t.report.privacy}</p>

				{#if status}
					<p class="cu-note" role="status">{status}</p>
				{/if}
				{#if failure}
					<p class="cu-error" role="alert">{failure}</p>
				{/if}

				<div class="cu-rb-actions">
					<button type="button" class="cu-btn primary" onclick={writeEmail}>{i18n.t.report.email}</button>
					<button type="button" class="cu-btn" onclick={openIssue}>{i18n.t.report.github}</button>
					<button type="button" class="cu-btn" onclick={copy}>{i18n.t.report.copy}</button>
				</div>
				<p class="cu-note">{i18n.t.report.githubNote}</p>
				<p class="cu-note">{i18n.t.report.security}</p>
			</div>
		</div>
	</div>
{/if}

<style>
	.cu-rb-overlay {
		position: fixed;
		inset: 0;
		background: var(--cu-overlay);
		display: flex;
		align-items: center;
		justify-content: center;
		z-index: 215;
		padding: 24px;
	}

	.cu-rb-modal {
		background: var(--cu-surface);
		border: 1px solid var(--cu-line);
		border-radius: var(--cu-radius-lg);
		box-shadow: var(--cu-shadow);
		width: 100%;
		max-width: 560px;
		max-height: 90vh;
		display: flex;
		flex-direction: column;
		overflow: hidden;
	}

	.cu-rb-head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 12px;
		padding: 14px 18px;
		border-bottom: 1px solid var(--cu-line);
	}

	.cu-rb-head h2 {
		margin: 0;
		font-size: 15px;
		font-weight: 600;
	}

	.cu-rb-body {
		padding: 16px 18px 20px;
		overflow-y: auto;
		display: flex;
		flex-direction: column;
		gap: 12px;
	}

	.cu-rb-body .cu-note,
	.cu-rb-body .cu-error {
		margin: 0;
	}

	.cu-rb-kinds {
		display: flex;
		gap: 8px;
	}

	.cu-rb-kind {
		flex: 1;
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 8px 12px;
		border: 1px solid var(--cu-line-strong);
		border-radius: var(--cu-radius-sm);
		font-size: 13px;
		cursor: pointer;
	}

	.cu-rb-kind.on {
		border-color: var(--cu-seal-text);
	}

	.cu-rb-body textarea {
		width: 100%;
		font: inherit;
		font-size: 13px;
		color: var(--cu-text);
		background: var(--cu-surface);
		border: 1px solid var(--cu-line-strong);
		border-radius: var(--cu-radius-sm);
		padding: 8px 10px;
		resize: vertical;
	}

	.cu-rb-check {
		display: flex;
		align-items: flex-start;
		gap: 8px;
		font-size: 13px;
	}

	.cu-rb-preview {
		font-family: var(--cu-mono, ui-monospace, monospace);
		font-size: 12px;
	}

	.cu-rb-actions {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
	}
</style>
