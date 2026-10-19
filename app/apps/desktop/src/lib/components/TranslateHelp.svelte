<script lang="ts">
	import { save as saveDialog } from '@tauri-apps/plugin-dialog';
	import { openUrl } from '@tauri-apps/plugin-opener';
	import { allLanguages, fill, i18n, loadLanguage, loadRawDictionary, previewTexts, showLanguage } from '$lib/i18n/index.svelte';
	import { flatten } from '$lib/i18n/core';
	import { en } from '$lib/i18n/locales/en';
	import {
		TRANSLATIONS_EMAIL,
		buildContribution,
		contributionFileName,
		editProblem,
		mailtoUrl,
	} from '$lib/i18n/contribute';
	import { translationExport } from '$lib/api/settings';
	import { friendlyError } from '$lib/errors';
	import { closeTranslateHelp, translateHelp } from '$lib/state/translateHelp.svelte';

	const PAGE = 40;
	const options = allLanguages.filter((l) => l.code !== 'en');
	const reference = flatten(en);
	const isPlural = (key: string) => /\.(zero|one|two|few|many|other)$/.test(key);

	let language = $state('');
	let current = $state.raw(new Map<string, string>());
	let edits = $state.raw(new Map<string, string>());
	let search = $state('');
	let onlyMine = $state(false);
	let limit = $state(PAGE);
	let credit = $state('');
	let contact = $state('');
	let nativeSpeaker = $state(false);
	let licence = $state(false);
	let status = $state<string | null>(null);
	let failure = $state<string | null>(null);
	let trying = $state(false);

	const info = $derived(options.find((l) => l.code === language));
	const storeKey = (code: string) => `cu-translate-${code}`;

	function remember() {
		try {
			if (edits.size) localStorage.setItem(storeKey(language), JSON.stringify([...edits]));
			else localStorage.removeItem(storeKey(language));
		} catch {
		}
	}

	async function choose(code: string) {
		language = code;
		limit = PAGE;
		status = null;
		failure = null;
		if (!code) return;
		current = flatten(await loadRawDictionary(code));
		let stored: [string, string][] = [];
		try {
			const raw = JSON.parse(localStorage.getItem(storeKey(code)) ?? '[]');
			if (Array.isArray(raw)) stored = raw.filter((e) => Array.isArray(e) && typeof e[0] === 'string' && typeof e[1] === 'string' && reference.has(e[0]));
		} catch {
			stored = [];
		}
		edits = new Map(stored);
	}

	function edit(key: string, value: string) {
		const next = new Map(edits);
		if (value.trim() === '' || value === (current.get(key) ?? '')) next.delete(key);
		else next.set(key, value);
		edits = next;
		remember();
		if (trying) previewTexts(edits);
	}

	const matching = $derived.by(() => {
		const q = search.trim().toLowerCase();
		const rows: { key: string; english: string; value: string; changed: boolean; problem: ReturnType<typeof editProblem> }[] = [];
		for (const [key, english] of reference) {
			const changed = edits.has(key);
			if (onlyMine && !changed) continue;
			const value = edits.get(key) ?? current.get(key) ?? '';
			if (q && !english.toLowerCase().includes(q) && !value.toLowerCase().includes(q)) continue;
			rows.push({ key, english, value, changed, problem: changed ? editProblem(english, value, isPlural(key)) : null });
		}
		return rows;
	});
	const shown = $derived(matching.slice(0, limit));
	const hasProblems = $derived([...edits].some(([key, value]) => editProblem(reference.get(key) ?? '', value, isPlural(key)) !== null));

	function contribution() {
		return buildContribution(language, edits, current, { nativeSpeaker, credit, contact, licenceAccepted: licence });
	}

	function ready(): boolean {
		status = null;
		failure = null;
		if (edits.size === 0) failure = i18n.t.translate.nothing;
		else if (hasProblems) failure = i18n.t.translate.fixFirst;
		return failure === null;
	}

	async function saveFile() {
		if (!ready()) return;
		try {
			const path = await saveDialog({
				defaultPath: contributionFileName(language),
				filters: [{ name: i18n.t.translate.fileTypeName, extensions: ['json'] }],
			});
			if (typeof path !== 'string') return;
			await translationExport(path, JSON.stringify(contribution(), null, 2));
			status = fill(i18n.t.translate.saved, { path, email: TRANSLATIONS_EMAIL });
		} catch (e) {
			failure = friendlyError(e);
		}
	}

	async function writeEmail() {
		if (!ready()) return;
		try {
			await openUrl(mailtoUrl(contribution()));
		} catch {
			failure = fill(i18n.t.translate.mailFailed, { email: TRANSLATIONS_EMAIL });
		}
	}

	function discard() {
		edits = new Map();
		remember();
		if (trying) previewTexts(edits);
	}

	async function toggleTrying() {
		if (trying) {
			trying = false;
			await loadLanguage();
		} else if (language) {
			await showLanguage(language);
			previewTexts(edits);
			trying = true;
		}
	}

	async function close() {
		if (trying) {
			trying = false;
			await loadLanguage();
		}
		closeTranslateHelp();
	}
</script>

{#if translateHelp.open}
	<div class="cu-tr-overlay">
		<div class="cu-tr-modal" role="dialog" aria-modal="true" aria-labelledby="cu-tr-title">
			<div class="cu-tr-head">
				<h2 id="cu-tr-title">{i18n.t.translate.title}</h2>
				<button type="button" class="cu-btn" onclick={close}>{i18n.t.common.close}</button>
			</div>
			<div class="cu-tr-body">
				<p class="cu-note">{i18n.t.translate.lead}</p>
				<label class="cu-field">
					<span>{i18n.t.translate.language}</span>
					<select value={language} onchange={(e) => choose(e.currentTarget.value)}>
						<option value="">{i18n.t.translate.choose}</option>
						{#each options as l (l.code)}
							<option value={l.code} lang={l.tag} dir={l.dir}>{l.name}</option>
						{/each}
					</select>
				</label>
				{#if info?.draft}
					<p class="cu-note">{i18n.t.translate.draftNote}</p>
				{/if}

				{#if info}
					<div class="cu-tr-tools">
						<label class="cu-field cu-tr-search">
							<span>{i18n.t.translate.search}</span>
							<input type="search" placeholder={i18n.t.translate.searchPlaceholder} bind:value={search} oninput={() => (limit = PAGE)} />
						</label>
						<label class="cu-tr-check">
							<input type="checkbox" bind:checked={onlyMine} onchange={() => (limit = PAGE)} />
							<span>{i18n.t.translate.onlyMine}</span>
						</label>
					</div>
					<p class="cu-note" aria-live="polite">
						{fill(i18n.t.translate.count, { edited: edits.size, shown: shown.length, total: matching.length })}
					</p>
					<p class="cu-note">{i18n.t.translate.rule}</p>

					<ul class="cu-tr-list">
						{#each shown as row (row.key)}
							<li class="cu-tr-row" class:changed={row.changed}>
								<p class="cu-tr-english" lang="en" dir="ltr">{row.english}</p>
								<textarea
									rows="2"
									lang={info.tag}
									dir={info.dir}
									aria-label={row.english}
									aria-invalid={row.problem !== null}
									value={row.value}
									oninput={(e) => edit(row.key, e.currentTarget.value)}
								></textarea>
								{#if row.problem}
									<p class="cu-error" role="alert">
										{row.problem === 'placeholders' ? i18n.t.translate.problemPlaceholders : i18n.t.translate.problemMarkup}
									</p>
								{/if}
							</li>
						{/each}
					</ul>
					{#if matching.length > shown.length}
						<button type="button" class="cu-btn" onclick={() => (limit += PAGE)}>{i18n.t.translate.showMore}</button>
					{/if}

					<h3>{i18n.t.translate.sendTitle}</h3>
					{#if trying}
						<p class="cu-note">{i18n.t.translate.trying}</p>
					{/if}
					<label class="cu-field">
						<span>{i18n.t.translate.credit}</span>
						<input type="text" maxlength="80" autocomplete="off" bind:value={credit} />
					</label>
					<label class="cu-field">
						<span>{i18n.t.translate.contact}</span>
						<input type="email" maxlength="120" autocomplete="off" bind:value={contact} />
					</label>
					<label class="cu-tr-check">
						<input type="checkbox" bind:checked={nativeSpeaker} />
						<span>{i18n.t.translate.native}</span>
					</label>
					<label class="cu-tr-check">
						<input type="checkbox" bind:checked={licence} />
						<span>{i18n.t.translate.licence}</span>
					</label>
					{#if status}
						<p class="cu-note" role="status">{status}</p>
					{/if}
					{#if failure}
						<p class="cu-error" role="alert">{failure}</p>
					{/if}
					<div class="cu-tr-actions">
						<button type="button" class="cu-btn primary" disabled={!licence} onclick={saveFile}>{i18n.t.translate.saveFile}</button>
						<button type="button" class="cu-btn" disabled={!licence} onclick={writeEmail}>{i18n.t.translate.writeEmail}</button>
						<button type="button" class="cu-btn" onclick={toggleTrying}>
							{trying ? i18n.t.translate.stopTrying : i18n.t.translate.tryIt}
						</button>
						<button type="button" class="cu-btn ghost" disabled={edits.size === 0} onclick={discard}>{i18n.t.translate.discard}</button>
					</div>
					<p class="cu-note">{fill(i18n.t.translate.sendNote, { email: TRANSLATIONS_EMAIL })}</p>
				{/if}
			</div>
		</div>
	</div>
{/if}

<style>
	.cu-tr-overlay {
		position: fixed;
		inset: 0;
		background: var(--cu-overlay);
		display: flex;
		align-items: center;
		justify-content: center;
		z-index: 210;
		padding: 24px;
	}

	.cu-tr-modal {
		background: var(--cu-surface);
		border: 1px solid var(--cu-line);
		border-radius: var(--cu-radius-lg);
		box-shadow: var(--cu-shadow);
		width: 100%;
		max-width: 760px;
		max-height: 90vh;
		display: flex;
		flex-direction: column;
		overflow: hidden;
	}

	.cu-tr-head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 12px;
		padding: 14px 18px;
		border-bottom: 1px solid var(--cu-line);
	}

	.cu-tr-head h2 {
		margin: 0;
		font-size: 15px;
		font-weight: 600;
	}

	.cu-tr-body {
		padding: 16px 18px 20px;
		overflow-y: auto;
		display: flex;
		flex-direction: column;
		gap: 12px;
	}

	.cu-tr-body h3 {
		margin: 12px 0 0;
		font-size: 14px;
	}

	.cu-tr-body .cu-note,
	.cu-tr-body .cu-error {
		margin: 0;
	}

	.cu-tr-tools {
		display: flex;
		align-items: flex-end;
		gap: 16px;
		flex-wrap: wrap;
	}

	.cu-tr-search {
		flex: 1 1 260px;
	}

	.cu-tr-check {
		display: flex;
		align-items: flex-start;
		gap: 8px;
		font-size: 13px;
	}

	.cu-tr-list {
		list-style: none;
		margin: 0;
		padding: 0;
		display: flex;
		flex-direction: column;
		gap: 10px;
	}

	.cu-tr-row {
		border: 1px solid var(--cu-line);
		border-radius: var(--cu-radius-sm);
		padding: 10px 12px;
		display: flex;
		flex-direction: column;
		gap: 6px;
	}

	.cu-tr-row.changed {
		border-color: var(--cu-seal-text);
	}

	.cu-tr-english {
		margin: 0;
		font-size: 13px;
		color: var(--cu-muted);
		white-space: pre-wrap;
	}

	.cu-tr-row textarea {
		width: 100%;
		font: inherit;
		font-size: 13px;
		color: var(--cu-text);
		background: var(--cu-surface);
		border: 1px solid var(--cu-line-strong);
		border-radius: var(--cu-radius-sm);
		padding: 6px 8px;
		resize: vertical;
	}

	.cu-tr-row textarea[aria-invalid='true'] {
		border-color: var(--cu-conflict);
	}

	.cu-tr-actions {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
	}
</style>
