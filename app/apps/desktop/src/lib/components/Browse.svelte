<script lang="ts">
	import Rich from './Rich.svelte';
	import { fill, i18n, plural } from '$lib/i18n/index.svelte';
	import { friendlyError } from '$lib/errors';
	import { session } from '$lib/state/session.svelte';
	import ConfirmDialog from './ConfirmDialog.svelte';
	import ClashDialog from './ClashDialog.svelte';
	import ShareDialog from './ShareDialog.svelte';
	import { open } from '@tauri-apps/plugin-dialog';
	import { getCurrentWebview } from '@tauri-apps/api/webview';
	import { basename, join, downloadDir } from '@tauri-apps/api/path';
	import {
		vaultList,
		vaultListRevealed,
		vaultMkdir,
		vaultImportMany,
		vaultExport,
		localPathKinds,
		vaultFindStream,
		vaultFindCancel,
		type Entry,
		type FoundEntry,
		type TransferEvent,
	} from '$lib/api/browse';
	import {
		vaultTrash,
		trashList,
		trashRestore,
		trashPurge,
		trashEmpty,
		type TrashEntry,
	} from '$lib/api/trash';
	import { vaultVersions, vaultRestoreVersion, type Version } from '$lib/api/versions';
	import { vaultShareFile } from '$lib/api/share';
	import { vaultSyncStates, type EntrySyncState } from '$lib/api/overview';
	import { formatBytes, formatWhen, uniqueName } from '$lib/format';
	import { notify } from '$lib/notify';

	const NOTIFY_MIN_DURATION_MS = 5_000;

	const RESERVED_ROOT_NAME = '.cryptoupload';

	type View = 'browse' | 'trash';
	let view = $state<View>('browse');

	let path = $state(session.browseDir ?? '');
	session.browseDir = null;
	let entries = $state<Entry[]>([]);
	let listedPath = $state('');
	let listRequest = 0;
	let revealRequest = 0;
	let deleteRequest = 0;
	let versionsRequest = 0;

	function entryPath(entry: Entry): string {
		return listedPath ? `${listedPath}/${entry.name}` : entry.name;
	}
	let loading = $state(true);
	let error = $state<string | null>(null);
	let transfer = $state<{
		label: string;
		file: string;
		bytes: number;
		index: number;
		total: number;
	} | null>(null);
	let creatingFolder = $state(false);
	let newFolderName = $state('');

	let searchQuery = $state('');
	let searchResults = $state<FoundEntry[] | null>(null);
	let searchTruncated = $state(false);
	let searching = $state(false);
	let searchStopped = $state(false);
	let searchError = $state<string | null>(null);

	let searchRun = 0;

	async function runSearch() {
		const pattern = searchQuery.trim();
		if (!pattern) return;
		const run = ++searchRun;
		searching = true;
		searchError = null;
		searchTruncated = false;
		searchStopped = false;
		searchResults = [];
		try {
			const outcome = await vaultFindStream(path, pattern, (found) => {
				if (run === searchRun) searchResults?.push(found);
			});
			if (run !== searchRun) return;
			searchTruncated = outcome.truncated;
			searchStopped = outcome.cancelled;
		} catch (e) {
			if (run === searchRun) searchError = friendlyError(e);
		} finally {
			if (run === searchRun) searching = false;
		}
	}

	function stopSearch() {
		void vaultFindCancel();
	}

	function closeSearch() {
		searchRun++;
		void vaultFindCancel();
		searching = false;
		searchResults = null;
		searchTruncated = false;
		searchStopped = false;
		searchError = null;
		searchQuery = '';
	}

	function openFound(found: FoundEntry) {
		const parts = found.path.split('/');
		path = found.isDir ? found.path : parts.slice(0, -1).join('/');
		closeSearch();
	}

	let confirmingDelete = $state<string | null>(null);
	let confirmingDeletePath = $state<string | null>(null);
	let deleteChildCount = $state<number | null>(null);

	let expandedVersions = $state<string | null>(null);
	let versionsList = $state<Version[]>([]);
	let versionsNoJob = $state(false);
	let versionsLoading = $state(false);
	let versionsError = $state<string | null>(null);
	let restoreMessage = $state<string | null>(null);

	let trashEntries = $state<TrashEntry[]>([]);
	let trashLoading = $state(false);
	let confirmingEmptyTrash = $state(false);

	let revealed = $state(false);
	let physicalDir = $state('');
	let encryptedNames = $state<Map<string, string>>(new Map());

	async function loadRevealed() {
		if (!revealed) return;
		const req = ++revealRequest;
		try {
			const listing = await vaultListRevealed(path);
			if (req !== revealRequest) return;
			physicalDir = listing.physicalDir;
			encryptedNames = new Map(listing.entries.map((e) => [e.name, e.encryptedName]));
		} catch (e) {
			if (req === revealRequest) error = friendlyError(e);
		}
	}

	$effect(() => {
		path;
		revealed;
		loadRevealed();
	});

	function segments(): string[] {
		return path ? path.split('/') : [];
	}

	async function load() {
		const req = ++listRequest;
		const target = path;
		loading = true;
		error = null;
		if (target !== listedPath) {
			expandedVersions = null;
			confirmingDelete = null;
			confirmingDeletePath = null;
			deleteChildCount = null;
			versionsRequest++;
			deleteRequest++;
		} else {
			expandedVersions = null;
		}
		try {
			const listed = await vaultList(target);
			if (req !== listRequest) return;
			entries = target === '' ? listed.filter((e) => e.name !== RESERVED_ROOT_NAME) : listed;
			listedPath = target;
		} catch (e) {
			if (req === listRequest) error = friendlyError(e);
		} finally {
			if (req === listRequest) loading = false;
		}
	}

	$effect(() => {
		load();
	});

	let syncStates = $state<Record<string, EntrySyncState>>({});

	$effect(() => {
		const dir = listedPath;
		const names = entries.map((e) => e.name);
		(async () => {
			try {
				const result = await vaultSyncStates(session.vaultRoot, dir, names);
				syncStates = result;
			} catch {
				syncStates = {};
			}
		})();
	});

	function switchView(next: View) {
		view = next;
		error = null;
		confirmingDelete = null;
		if (next === 'trash') loadTrash();
	}

	function enter(name: string) {
		path = listedPath ? `${listedPath}/${name}` : name;
	}

	function goTo(index: number) {
		path = segments()
			.slice(0, index + 1)
			.join('/');
	}

	function goToRoot() {
		path = '';
	}

	function onTransferEvent(event: TransferEvent) {
		if (!transfer) return;
		if (event.kind === 'item') {
			transfer = { ...transfer, index: event.index + 1 };
		} else if (event.kind === 'file') {
			transfer = { ...transfer, file: event.vaultPath, bytes: 0 };
		} else {
			transfer = { ...transfer, bytes: event.count };
		}
	}

	type ClashChoice = 'both' | 'replace' | 'skip';
	let clash = $state<{
		name: string;
		isDir: boolean;
		canReplace: boolean;
		remaining: number;
		resolve: (answer: { choice: ClashChoice; all: boolean }) => void;
	} | null>(null);
	let dragging = $state(false);

	function askClash(name: string, isDir: boolean, canReplace: boolean, remaining: number) {
		return new Promise<{ choice: ClashChoice; all: boolean }>((resolve) => {
			clash = {
				name,
				isDir,
				canReplace,
				remaining,
				resolve: (answer) => {
					clash = null;
					resolve(answer);
				},
			};
		});
	}

	let importBusy = false;
	async function importMany(paths: string[]) {
		if (paths.length === 0 || view !== 'browse') return;
		if (importBusy || transfer || clash) {
			error = i18n.t.browse.uploadBusy;
			return;
		}
		importBusy = true;
		try {
			await importManyInner(paths);
		} finally {
			importBusy = false;
		}
	}

	async function importManyInner(paths: string[]) {
		error = null;
		const destDir = listedPath;
		let kinds: boolean[];
		try {
			kinds = await localPathKinds(paths);
		} catch (e) {
			error = friendlyError(e);
			return;
		}
		const taken = new Map<string, 'file' | 'dir' | 'queued'>(
			entries.map((e) => [e.name.toLowerCase(), e.kind]),
		);
		const takenNames = new Set(taken.keys());
		let sticky: ClashChoice | null = null;
		const startedAt = Date.now();
		const items: { localSrc: string; vaultDest: string; label: string; isDir: boolean }[] = [];
		for (let i = 0; i < paths.length; i++) {
			const src = paths[i];
			const isDir = kinds[i];
			const original = await basename(src);
			let name = original;
			if (taken.has(original.toLowerCase())) {
				const existing = taken.get(original.toLowerCase());
				const canReplace = !isDir && existing === 'file';
				let choice: ClashChoice;
				if (sticky !== null && (sticky !== 'replace' || canReplace)) {
					choice = sticky;
				} else {
					const answer = await askClash(original, isDir, canReplace, paths.length - i - 1);
					choice = answer.choice;
					if (answer.all) sticky = choice;
				}
				if (choice === 'skip') continue;
				if (choice === 'both' || !canReplace) {
					name = uniqueName(original, takenNames, isDir);
				}
			}
			taken.set(name.toLowerCase(), 'queued');
			takenNames.add(name.toLowerCase());
			items.push({ localSrc: src, vaultDest: destDir ? `${destDir}/${name}` : name, label: original, isDir });
		}
		if (items.length === 0) return;
		transfer = {
			label: items.length === 1 && items[0].isDir ? i18n.t.browse.uploadingFolder : i18n.t.browse.uploading,
			file: items[0].label,
			bytes: 0,
			index: 1,
			total: items.length,
		};
		let uploaded = 0;
		const failed: string[] = [];
		try {
			const results = await vaultImportMany(
				items.map(({ localSrc, vaultDest }) => ({ localSrc, vaultDest })),
				onTransferEvent,
			);
			results.forEach((message, i) => {
				if (message === null) uploaded++;
				else failed.push(`${items[i].label}: ${friendlyError(message)}`);
			});
		} catch (e) {
			failed.push(friendlyError(e));
		} finally {
			transfer = null;
		}
		await load();
		if (failed.length > 0) {
			error =
				(uploaded > 0 ? fill(i18n.t.browse.uploadSummary, { uploaded, failed: failed.length }) + ' ' : '') + failed.join(' · ');
		}
		if (Date.now() - startedAt >= NOTIFY_MIN_DURATION_MS) {
			const summary = items.length === 1 ? items[0].label : plural(uploaded, i18n.t.browse.itemsCount);
			if (failed.length > 0) notify(fill(i18n.t.browse.notifyUploadPartial, { summary }), error ?? '');
			else notify(fill(i18n.t.browse.notifyUploadDone, { summary }), i18n.t.browse.nowInVault);
		}
	}

	async function pickAndImport(directory: boolean) {
		const picked = await open({ directory, multiple: true });
		if (picked === null) return;
		await importMany(Array.isArray(picked) ? picked : [picked]);
	}

	function handleDrag(p: { type: string; paths?: string[] }) {
		if (p.type === 'enter' || p.type === 'over') dragging = view === 'browse';
		else if (p.type === 'leave') dragging = false;
		else if (p.type === 'drop') {
			dragging = false;
			void importMany(p.paths ?? []);
		}
	}

	$effect(() => {
		let unlisten: (() => void) | undefined;
		let cancelled = false;
		getCurrentWebview()
			.onDragDropEvent((event) => handleDrag(event.payload))
			.then((fn) => (cancelled ? fn() : (unlisten = fn)))
			.catch(() => {});
		return () => {
			cancelled = true;
			unlisten?.();
		};
	});

	let sharing = $state<{ name: string; vaultPath: string; defaultOut: string } | null>(null);
	let shareBusy = $state(false);
	let shareError = $state<string | null>(null);
	let shareNeedsOverwrite = $state(false);
	let shareDone = $state<{ out: string; name: string } | null>(null);

	async function startShare(entry: Entry) {
		shareError = null;
		shareNeedsOverwrite = false;
		shareDone = null;
		let defaultOut = `${entry.name}.cushare`;
		try {
			defaultOut = await join(await downloadDir(), defaultOut);
		} catch {
		}
		sharing = { name: entry.name, vaultPath: entryPath(entry), defaultOut };
	}

	async function createShare(args: { password: string; out: string; overwrite: boolean }) {
		if (!sharing || shareBusy) return;
		const target = sharing;
		shareBusy = true;
		shareError = null;
		error = null;
		transfer = { label: i18n.t.browse.creatingPackage, file: target.name, bytes: 0, index: 1, total: 1 };
		try {
			await vaultShareFile(target.vaultPath, args.out, args.password, args.overwrite, onTransferEvent);
			shareDone = { out: args.out, name: target.name };
			sharing = null;
		} catch (e) {
			const message = friendlyError(e);
			shareNeedsOverwrite = String(e) === 'DESTINATION_EXISTS';
			shareError = message;
		} finally {
			shareBusy = false;
			transfer = null;
		}
	}

	async function downloadEntry(entry: Entry) {
		const destDir = await open({ directory: true, multiple: false });
		if (typeof destDir !== 'string') return;
		const vaultSrc = entryPath(entry);
		const localDest = await join(destDir, entry.name);
		error = null;
		transfer = { label: i18n.t.browse.downloading, file: entry.name, bytes: 0, index: 1, total: 1 };
		const startedAt = Date.now();
		try {
			await vaultExport(vaultSrc, localDest, onTransferEvent);
			if (Date.now() - startedAt >= NOTIFY_MIN_DURATION_MS) {
				notify(fill(i18n.t.browse.notifyDownloadDone, { name: entry.name }), fill(i18n.t.recoveryKit.savedTo, { path: destDir }));
			}
		} catch (e) {
			const message = friendlyError(e);
			error = message;
			if (Date.now() - startedAt >= NOTIFY_MIN_DURATION_MS) {
				notify(fill(i18n.t.browse.notifyDownloadFailed, { name: entry.name }), message);
			}
		} finally {
			transfer = null;
		}
	}

	async function createFolder() {
		const name = newFolderName.trim();
		if (!name) return;
		const dest = path ? `${path}/${name}` : name;
		try {
			await vaultMkdir(dest);
			creatingFolder = false;
			newFolderName = '';
			await load();
		} catch (e) {
			error = friendlyError(e);
		}
	}

	async function countChildren(vaultPath: string): Promise<number> {
		const children = await vaultList(vaultPath);
		let count = 0;
		for (const child of children) {
			count++;
			if (child.kind === 'dir') {
				count += await countChildren(`${vaultPath}/${child.name}`);
			}
		}
		return count;
	}

	async function askDelete(entry: Entry) {
		error = null;
		const req = ++deleteRequest;
		const vaultPath = entryPath(entry);
		confirmingDelete = entry.name;
		confirmingDeletePath = vaultPath;
		deleteChildCount = null;
		if (entry.kind === 'dir') {
			try {
				const count = await countChildren(vaultPath);
				if (req === deleteRequest) deleteChildCount = count;
			} catch (e) {
				if (req === deleteRequest) error = friendlyError(e);
			}
		}
	}

	function cancelDelete() {
		deleteRequest++;
		confirmingDelete = null;
		confirmingDeletePath = null;
		deleteChildCount = null;
	}

	async function confirmDelete() {
		const vaultPath = confirmingDeletePath;
		if (!vaultPath) return;
		try {
			await vaultTrash(vaultPath);
			cancelDelete();
			await load();
		} catch (e) {
			error = friendlyError(e);
		}
	}

	async function toggleVersions(entry: Entry) {
		if (expandedVersions === entry.name) {
			expandedVersions = null;
			return;
		}
		const req = ++versionsRequest;
		expandedVersions = entry.name;
		versionsLoading = true;
		versionsError = null;
		versionsNoJob = false;
		versionsList = [];
		restoreMessage = null;
		try {
			const result = await vaultVersions(entryPath(entry));
			if (req !== versionsRequest) return;
			versionsNoJob = result.noJobConfigured;
			versionsList = result.versions;
		} catch (e) {
			if (req === versionsRequest) versionsError = friendlyError(e);
		} finally {
			if (req === versionsRequest) versionsLoading = false;
		}
	}

	async function restoreVersionTo(entry: Entry, v: Version) {
		versionsError = null;
		restoreMessage = null;
		try {
			const destDir = await open({ directory: true, multiple: false });
			if (typeof destDir !== 'string') return;
			const localDest = await join(destDir, v.name);
			const vaultPath = entryPath(entry);
			await vaultRestoreVersion(vaultPath, v.version, localDest);
			restoreMessage = fill(i18n.t.browse.versionRestored, { version: v.version, path: localDest });
		} catch (e) {
			versionsError = friendlyError(e);
		}
	}

	async function loadTrash() {
		trashLoading = true;
		error = null;
		try {
			trashEntries = await trashList();
		} catch (e) {
			error = friendlyError(e);
		} finally {
			trashLoading = false;
		}
	}

	async function restore(entry: TrashEntry) {
		error = null;
		try {
			await trashRestore(entry.id);
			await loadTrash();
			await load();
		} catch (e) {
			error = friendlyError(e);
		}
	}

	let confirmingPurge = $state<string | null>(null);

	async function purge(entry: TrashEntry) {
		confirmingPurge = null;
		error = null;
		try {
			await trashPurge(entry.id);
			await loadTrash();
		} catch (e) {
			error = friendlyError(e);
		}
	}

	async function doEmptyTrash() {
		try {
			await trashEmpty();
			confirmingEmptyTrash = false;
			await loadTrash();
		} catch (e) {
			error = friendlyError(e);
		}
	}
</script>

<div class="cu-page">
	<h1 class="cu-page-title cu-browse-title">
		{view === 'trash' ? i18n.t.browse.trash : listedPath ? listedPath.split('/').pop() : i18n.t.nav.browse}
	</h1>
	<div class="cu-view-tabs" role="tablist">
		<button
			role="tab"
			aria-selected={view === 'browse'}
			class:active={view === 'browse'}
			onclick={() => switchView('browse')}
		>
			{i18n.t.nav.browse}
		</button>
		<button
			role="tab"
			aria-selected={view === 'trash'}
			class:active={view === 'trash'}
			onclick={() => switchView('trash')}
		>
			{i18n.t.browse.trash}
		</button>
	</div>

	{#if view === 'browse'}
		<div class="cu-toolbar">
			<nav class="cu-breadcrumb" aria-label={i18n.t.browse.pathAria}>
				<button type="button" onclick={goToRoot}>{i18n.t.browse.rootCrumb}</button>
				{#each segments() as seg, i (i)}
					<span class="cu-sep">/</span>
					<button type="button" onclick={() => goTo(i)}>{seg}</button>
				{/each}
			</nav>
			<div class="cu-actions">
				<button
					type="button"
					class="cu-reveal-toggle cu-btn"
					class:active={revealed}
					aria-pressed={revealed}
					onclick={() => (revealed = !revealed)}
				>
					{revealed ? i18n.t.browse.showNames : i18n.t.browse.showEncrypted}
				</button>
				<button
					type="button"
					class="cu-btn"
					onclick={() => (creatingFolder = !creatingFolder)}
				>
					{i18n.t.browse.newFolder}
				</button>
				<button type="button" class="cu-btn" onclick={() => pickAndImport(false)}>
					{i18n.t.browse.uploadFiles}
				</button>
				<button type="button" class="cu-btn" onclick={() => pickAndImport(true)}>
					{i18n.t.browse.uploadFolder}
				</button>
			</div>
		</div>

		<form
			class="cu-new-folder"
			onsubmit={(e) => {
				e.preventDefault();
				runSearch();
			}}
		>
			<input
				type="text"
				bind:value={searchQuery}
				placeholder={i18n.t.browse.searchPlaceholder}
				aria-label={i18n.t.browse.searchAria}
			/>
			<button
				type="submit"
				class="cu-btn primary"
				disabled={searching || !searchQuery.trim()}
			>
				{searching ? i18n.t.browse.searching : i18n.t.browse.search}
			</button>
			{#if searching}
				<button type="button" class="cu-btn" onclick={stopSearch}>{i18n.t.common.stop}</button>
			{/if}
			{#if searchResults !== null}
				<button type="button" class="cu-btn" onclick={closeSearch}>
					{i18n.t.browse.closeSearch}
				</button>
			{/if}
		</form>

		{#if searchError}
			<p class="cu-error" role="alert">{searchError}</p>
		{/if}

		{#if searchResults !== null}
			<div>
				{#if searching}
					<p class="cu-note">
						{plural(searchResults.length, i18n.t.browse.searchingSoFar)}
					</p>
				{/if}
				{#if searchResults.length === 0}
					{#if !searching}<p class="cu-note">{searchStopped ? i18n.t.browse.searchStoppedEmpty : i18n.t.browse.noResults}</p>{/if}
				{:else}
					<ul class="cu-row-list">
						{#each searchResults as found (found.path)}
							<li class="cu-row cu-row-simple">
								<span class="cu-row-name">
									<svg class="cu-i"><use href={found.isDir ? '#cu-i-folder' : '#cu-i-file'} /></svg>
									<button
										type="button"
										class="cu-name-btn cu-row-name-text"
										title={found.path}
										onclick={() => openFound(found)}
									>
										{found.path}{found.isDir ? '/' : ''}
									</button>
								</span>
							</li>
						{/each}
					</ul>
					{#if searchStopped}
						<p class="cu-note">
							{fill(i18n.t.browse.searchStopped, { n: searchResults.length })}
						</p>
					{/if}
					{#if searchTruncated}
						<p class="cu-note">
							{fill(i18n.t.browse.searchTruncated, { n: searchResults.length })}
						</p>
					{/if}
				{/if}
			</div>
		{/if}

		{#if revealed}
			<p class="cu-physical-note cu-mono">
				{i18n.t.browse.onDisk} <span class="cu-physical-path">{physicalDir || i18n.t.browse.vaultRoot}</span>
			</p>
		{/if}

		{#if creatingFolder}
			<form
				class="cu-new-folder"
				onsubmit={(e) => {
					e.preventDefault();
					createFolder();
				}}
			>
				<input
					type="text"
					bind:value={newFolderName}
					placeholder={i18n.t.browse.folderNamePlaceholder}
				/>
				<button type="submit" class="cu-btn primary">
					{i18n.t.common.create}
				</button>
			</form>
		{/if}

		{#if transfer}
			<div class="cu-transfer">
				<span>
					{transfer.label}{transfer.total > 1 ? ` ${fill(i18n.t.browse.transferStep, { index: transfer.index, total: transfer.total })}` : ''}:
					<span class="cu-mono">{transfer.file}</span>
				</span>
				<span class="cu-mono">{formatBytes(transfer.bytes)}</span>
			</div>
		{/if}

		{#if shareDone}
			<p class="cu-note cu-share-done" role="status">
				<Rich text={i18n.t.browse.packageCreated} values={{ path: shareDone.out }} />
				<button type="button" class="cu-btn sm" onclick={() => (shareDone = null)}>{i18n.t.common.close}</button>
			</p>
		{/if}

		{#if error}
			<p class="cu-error" role="alert">{error}</p>
		{/if}

		{#if loading && searchResults === null}
			<p class="cu-note">{i18n.t.common.loading}</p>
		{:else if entries.length === 0 && searchResults === null}
			<div class="cu-empty">
				<h2>{i18n.t.browse.emptyFolder}</h2>
				<p>{i18n.t.browse.emptyHint}</p>
				<div class="cu-empty-actions">
					<button
						type="button"
						class="cu-btn primary"
						onclick={() => pickAndImport(false)}
					>
						{i18n.t.browse.uploadFiles}
					</button>
					<button
						type="button"
						class="cu-btn"
						onclick={() => pickAndImport(true)}
					>
						{i18n.t.browse.uploadFolder}
					</button>
				</div>
			</div>
		{:else if searchResults === null}
			<ul class="cu-row-list">
				<li class="cu-row-head" aria-hidden="true">
					<span>{i18n.t.browse.colName}</span>
					<span>{i18n.t.browse.colSize}</span>
					<span>{i18n.t.browse.colModified}</span>
					<span>{i18n.t.browse.colState}</span>
					<span></span>
				</li>
				{#each entries as entry (entry.name)}
					{@const syncState = syncStates[entry.name]}
					<li class="cu-row-group">
						<div class="cu-row">
							<span class="cu-row-name">
								<svg class="cu-i"><use href={entry.kind === 'dir' ? '#cu-i-folder' : '#cu-i-file'} /></svg>
								{#if revealed}
									{@const encName = encryptedNames.get(entry.name)}
									{#if entry.kind === 'dir'}
										<button
											type="button"
											class="cu-name-btn cu-mono cu-encrypted cu-row-name-text"
											onclick={() => enter(entry.name)}
										>
											{encName ? `${encName}.uvf/` : '…'}
										</button>
									{:else}
										<span class="cu-mono cu-encrypted cu-row-name-text">{encName ? `${encName}.uvf` : '…'}</span>
									{/if}
								{:else if entry.kind === 'dir'}
									<button
										type="button"
										class="cu-name-btn cu-row-name-text"
										title={entry.name}
										onclick={() => enter(entry.name)}
									>
										{entry.name}
									</button>
								{:else}
									<span class="cu-row-name-text" title={entry.name}>{entry.name}</span>
								{/if}
							</span>
							<span class="cu-row-size">{entry.size === null ? '—' : formatBytes(entry.size)}</span>
							<span class="cu-row-modified">{entry.modified === null ? '—' : formatWhen(entry.modified)}</span>
							<span
								class="cu-row-status"
								class:synced={syncState === 'synced'}
								class:pending={syncState === 'pending'}
								class:conflict={syncState === 'conflict'}
							>
								{#if syncState === 'synced'}
									<svg class="cu-i"><use href="#cu-i-check" /></svg>{i18n.t.home.stateUpToDate}
								{:else if syncState === 'pending'}
									<svg class="cu-i"><use href="#cu-i-sync" /></svg>{i18n.t.browse.statePending}
								{:else if syncState === 'conflict'}
									<svg class="cu-i"><use href="#cu-i-copy" /></svg>{i18n.t.browse.stateTwoVersions}
								{/if}
							</span>
							<span
								class="cu-row-actions"
								class:cu-force-visible={confirmingDelete === entry.name}
							>
								<button type="button" class="cu-btn sm" onclick={() => downloadEntry(entry)}>{i18n.t.browse.download}</button>
								{#if entry.kind === 'file'}
									<button
										type="button"
										class="cu-btn sm"
										onclick={() => toggleVersions(entry)}
									>
										{expandedVersions === entry.name ? i18n.t.browse.hideVersions : i18n.t.browse.versions}
									</button>
									<button
										type="button"
										class="cu-btn sm"
										onclick={() => startShare(entry)}
									>
										{i18n.t.browse.share}
									</button>
								{/if}
								{#if confirmingDelete === entry.name}
									<button
										type="button"
										class="cu-btn sm danger"
										onclick={confirmDelete}
									>
										{i18n.t.common.confirm}
									</button>
									<button type="button" class="cu-btn sm" onclick={cancelDelete}>{i18n.t.common.cancel}</button>
								{:else}
									<button
										type="button"
										class="cu-btn sm"
										onclick={() => askDelete(entry)}
									>
										{i18n.t.common.delete}
									</button>
								{/if}
							</span>
						</div>
						{#if confirmingDelete === entry.name}
							<div class="cu-confirm-row">
								<p class="cu-note">
									{#if entry.kind === 'dir'}
										{#if deleteChildCount === null}
											{i18n.t.browse.countingContent}
										{:else if deleteChildCount === 0}
											{i18n.t.browse.deleteEmptyFolder}
										{:else}
											{plural(deleteChildCount, i18n.t.browse.deleteFolderWithContent)}
										{/if}
									{:else}
										{i18n.t.browse.deleteFile}
									{/if}
								</p>
							</div>
						{/if}
						{#if expandedVersions === entry.name}
							<div class="cu-versions-row">
								{#if versionsLoading}
									<p class="cu-note">{i18n.t.browse.loadingVersions}</p>
								{:else if versionsError}
									<p class="cu-error" role="alert">{versionsError}</p>
								{:else if versionsNoJob}
									<p class="cu-note">
										{i18n.t.browse.versionsNeedJob}
									</p>
								{:else if versionsList.length === 0}
									<p class="cu-note">{i18n.t.browse.noVersions}</p>
								{:else}
									<ul class="cu-version-list">
										{#each versionsList as v (v.version)}
											<li class="cu-version-row">
												<span class="cu-mono">v{v.version}</span>
												<span>{new Date(v.createdAt).toLocaleString(i18n.language.tag)}</span>
												<span class="cu-mono">{formatBytes(v.size)}</span>
												<span>
													{#if v.deleted}
														{i18n.t.browse.versionDeleted}
													{:else if v.location === 'canonical'}
														{i18n.t.browse.versionCurrent}
													{:else}
														{i18n.t.browse.versionArchived}
													{/if}
												</span>
												<span class="cu-row-actions">
													<button
														type="button"
														class="cu-btn sm"
														disabled={v.deleted}
														onclick={() => restoreVersionTo(entry, v)}
													>
														{i18n.t.browse.restoreMore}
													</button>
												</span>
											</li>
										{/each}
									</ul>
								{#if restoreMessage}
									<p class="cu-note">{restoreMessage}</p>
								{/if}
								{/if}
							</div>
						{/if}
					</li>
				{/each}
			</ul>
		{/if}
	{:else}
		<div class="cu-toolbar">
			<p class="cu-note cu-trash-hint">
				{i18n.t.browse.trashNote}
			</p>
			<div class="cu-actions">
				<button
					type="button"
					class="cu-btn danger"
					disabled={trashEntries.length === 0}
					onclick={() => (confirmingEmptyTrash = true)}
				>
					<svg class="cu-i"><use href="#cu-i-trash" /></svg>
					{i18n.t.browse.emptyTrash}
				</button>
			</div>
		</div>

		{#if confirmingEmptyTrash}
			<ConfirmDialog
				title={i18n.t.browse.emptyTrashTitle}
				description={plural(trashEntries.length, i18n.t.browse.emptyTrashMessage)}
				confirmLabel={plural(trashEntries.length, i18n.t.browse.emptyTrashConfirm)}
				typedWord={i18n.t.browse.emptyTrashWord}
				onconfirm={doEmptyTrash}
				oncancel={() => (confirmingEmptyTrash = false)}
			/>
		{/if}

		{#if error}
			<p class="cu-error" role="alert">{error}</p>
		{/if}

		{#if trashLoading}
			<p class="cu-note">{i18n.t.common.loading}</p>
		{:else if trashEntries.length === 0}
			<p class="cu-note">{i18n.t.browse.trashEmpty}</p>
		{:else}
			<ul class="cu-row-list">
				<li class="cu-row-head" aria-hidden="true">
					<span>{i18n.t.browse.colName}</span>
					<span>{i18n.t.browse.colSize}</span>
					<span>{i18n.t.browse.colDeleted}</span>
					<span>{i18n.t.browse.colState}</span>
					<span></span>
				</li>
				{#each trashEntries as entry (entry.id)}
					<li class="cu-row-group">
						<div class="cu-row">
							<span class="cu-row-name">
								<svg class="cu-i"><use href={entry.kind === 'dir' ? '#cu-i-folder' : '#cu-i-file'} /></svg>
								<span class="cu-row-name-text" title={entry.name}>{entry.name}</span>
								<span class="cu-trash-origin">
									{entry.originalPath ? fill(i18n.t.browse.trashFrom, { path: entry.originalPath }) : i18n.t.browse.originalPathUnknown}
								</span>
							</span>
							<span class="cu-row-size">—</span>
							<span class="cu-row-modified">{formatWhen(entry.deletedAt)}</span>
							<span class="cu-row-status">
								<svg class="cu-i"><use href="#cu-i-trash" /></svg>{i18n.t.browse.inTrash}
							</span>
							<span
								class="cu-row-actions"
								class:cu-force-visible={confirmingPurge === entry.id}
							>
								<button
									type="button"
									class="cu-btn sm"
									onclick={() => restore(entry)}
								>
									{i18n.t.browse.restore}
								</button>
								{#if confirmingPurge === entry.id}
									<button
										type="button"
										class="cu-btn sm danger"
										onclick={() => purge(entry)}
									>
										{i18n.t.browse.confirmDeleteForever}
									</button>
									<button type="button" class="cu-btn sm" onclick={() => (confirmingPurge = null)}>{i18n.t.common.cancel}</button>
								{:else}
									<button
										type="button"
										class="cu-btn sm"
										onclick={() => (confirmingPurge = entry.id)}
									>
										{i18n.t.browse.deleteForever}
									</button>
								{/if}
							</span>
						</div>
					</li>
				{/each}
			</ul>
		{/if}
	{/if}

	{#if sharing}
		<ShareDialog
			fileName={sharing.name}
			defaultOut={sharing.defaultOut}
			busy={shareBusy}
			error={shareError}
			mustConfirmOverwrite={shareNeedsOverwrite}
			oncreate={createShare}
			oncancel={() => (sharing = null)}
		/>
	{/if}

	{#if clash}
		<ClashDialog
			name={clash.name}
			isDir={clash.isDir}
			canReplace={clash.canReplace}
			remaining={clash.remaining}
			onchoose={(choice, all) => clash?.resolve({ choice, all })}
		/>
	{/if}

	{#if dragging}
		<div class="cu-drop-overlay" aria-hidden="true">
			<div class="cu-drop-hint">
				<svg class="cu-i"><use href="#cu-i-up" /></svg>
				<span>{fill(i18n.t.browse.dropHere, { folder: (listedPath ? listedPath.split('/').pop() : i18n.t.browse.rootCrumb) ?? i18n.t.browse.rootCrumb })}</span>
			</div>
		</div>
	{/if}
</div>

<style>
	.cu-page {
		padding: 30px 40px 26px;
	}

	.cu-view-tabs {
		display: flex;
		gap: 4px;
		margin-bottom: 16px;
		border-bottom: 1px solid var(--cu-line);
	}

	.cu-view-tabs button {
		background: none;
		border: none;
		border-bottom: 2px solid transparent;
		color: var(--cu-muted);
		font: inherit;
		font-size: 13px;
		padding: 8px 4px;
		margin-inline-end: 20px;
		cursor: pointer;
	}

	.cu-view-tabs button.active {
		color: var(--cu-text);
		border-bottom-color: var(--cu-seal-text);
		font-weight: 600;
	}

	.cu-toolbar {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 16px;
		margin-bottom: 16px;
		flex-wrap: wrap;
	}

	.cu-breadcrumb {
		font-size: 13px;
		color: var(--cu-muted);
	}

	.cu-browse-title {
		margin-bottom: 4px;
		word-break: break-word;
	}

	.cu-breadcrumb button {
		background: none;
		border: none;
		color: var(--cu-muted);
		font: inherit;
		cursor: pointer;
		min-height: 24px;
		padding: 2px 6px;
		border-radius: var(--cu-radius-sm);
	}

	.cu-breadcrumb button:hover {
		background: var(--cu-surface-raised);
	}

	.cu-breadcrumb button:last-child {
		color: var(--cu-text);
	}

	.cu-sep {
		margin: 0 2px;
		color: var(--cu-muted);
	}

	.cu-trash-hint {
		margin: 0;
	}

	.cu-actions {
		display: flex;
		align-items: center;
		gap: 8px;
	}

	.cu-reveal-toggle.cu-btn {
		border-color: var(--cu-seal-text);
		color: var(--cu-seal-text);
	}

	.cu-reveal-toggle.cu-btn.active {
		background: var(--cu-seal);
		border-color: var(--cu-seal);
		color: var(--cu-on-seal);
		font-weight: 500;
	}

	.cu-physical-note {
		color: var(--cu-muted);
		font-size: 12px;
		margin: -8px 0 16px;
	}

	.cu-physical-path {
		color: var(--cu-seal-text);
	}

	.cu-encrypted {
		letter-spacing: 0.01em;
	}

	.cu-new-folder {
		display: flex;
		gap: 8px;
		margin-bottom: 16px;
		max-width: 320px;
	}

	.cu-new-folder input {
		flex: 1;
		height: 34px;
		background: var(--cu-bg);
		border: 1px solid var(--cu-line-strong);
		border-radius: var(--cu-radius-sm);
		padding: 0 12px;
		color: var(--cu-text);
	}

	.cu-transfer {
		display: flex;
		justify-content: space-between;
		background: var(--cu-surface);
		border: 1px solid var(--cu-seal);
		border-radius: var(--cu-radius-sm);
		padding: 9px 14px;
		font-size: 13px;
		margin-bottom: 16px;
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

	.cu-note {
		color: var(--cu-muted);
		font-size: 13px;
	}

	.cu-confirm-row {
		padding: 0 12px 10px;
		border-bottom: 1px solid var(--cu-line);
	}

	.cu-confirm-row .cu-note {
		margin: 0;
	}

	.cu-versions-row {
		padding: 8px 12px 12px;
		border-bottom: 1px solid var(--cu-line);
	}

	.cu-versions-row .cu-note {
		margin: 0;
	}

	.cu-version-list {
		list-style: none;
		margin: 0 0 4px;
		padding: 0;
	}

	.cu-version-row {
		display: grid;
		grid-template-columns: 56px 1fr 90px 90px 130px;
		align-items: center;
		column-gap: 12px;
		min-height: 32px;
		padding: 0 8px;
		border-bottom: 1px solid var(--cu-line);
		border-radius: var(--cu-radius-sm);
		font-size: 12px;
		color: var(--cu-muted);
		position: relative;
	}

	.cu-version-row:hover,
	.cu-version-row:focus-within {
		background: var(--cu-surface);
	}

	.cu-version-row::before {
		content: '';
		position: absolute;
		inset-inline-start: 0;
		top: 0;
		bottom: 0;
		width: 2px;
		background: var(--cu-seal-text);
		opacity: 0;
	}

	.cu-version-row:hover::before,
	.cu-version-row:focus-within::before {
		opacity: 1;
	}

	.cu-row-list {
		list-style: none;
		margin: 0;
		padding: 0;
	}

	.cu-row-head {
		display: grid;
		grid-template-columns: minmax(0, 1fr) 84px 112px 130px;
		align-items: center;
		column-gap: 14px;
		min-height: 28px;
		padding: 0 10px 0 12px;
		border-bottom: 1px solid var(--cu-line-strong);
		color: var(--cu-muted);
		font-size: 12px;
	}

	.cu-row-group {
		position: relative;
	}

	.cu-row {
		display: grid;
		grid-template-columns: minmax(0, 1fr) 84px 112px 130px;
		align-items: center;
		column-gap: 14px;
		min-height: 40px;
		padding: 0 10px 0 12px;
		border-bottom: 1px solid var(--cu-line);
		position: relative;
	}

	.cu-row-simple {
		display: flex;
		align-items: center;
		min-height: 36px;
		padding: 0 10px 0 12px;
		border-bottom: 1px solid var(--cu-line);
		position: relative;
	}

	.cu-row:hover,
	.cu-row:focus-within,
	.cu-row-simple:hover,
	.cu-row-simple:focus-within {
		background: var(--cu-surface);
	}

	.cu-row::before,
	.cu-row-simple::before {
		content: '';
		position: absolute;
		inset-inline-start: 0;
		top: 0;
		bottom: 0;
		width: 2px;
		background: var(--cu-seal-text);
		opacity: 0;
	}

	.cu-row:hover::before,
	.cu-row:focus-within::before,
	.cu-row-simple:hover::before,
	.cu-row-simple:focus-within::before {
		opacity: 1;
	}

	.cu-row-name {
		display: flex;
		align-items: center;
		gap: 10px;
		min-width: 0;
	}

	.cu-row-name .cu-i {
		width: 16px;
		height: 16px;
		flex-shrink: 0;
		color: var(--cu-muted);
	}

	.cu-row-name-text {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		min-width: 0;
	}

	.cu-row-size,
	.cu-row-modified {
		color: var(--cu-muted);
		font-size: 12px;
		white-space: nowrap;
		font-variant-numeric: tabular-nums;
	}

	.cu-row-status {
		display: flex;
		align-items: center;
		gap: 6px;
		font-size: 12px;
		white-space: nowrap;
		color: var(--cu-muted);
	}

	.cu-row-status .cu-i {
		width: 14px;
		height: 14px;
		flex-shrink: 0;
	}

	.cu-row-status.synced {
		color: var(--cu-verified);
	}

	.cu-row-status.pending {
		color: var(--cu-seal-text);
	}

	.cu-row-status.conflict {
		color: var(--cu-warn);
	}

	.cu-name-btn {
		min-height: 24px;
		background: none;
		border: none;
		color: var(--cu-text);
		font: inherit;
		cursor: pointer;
		padding: 0;
		text-decoration: underline;
		text-decoration-color: var(--cu-line);
	}

	.cu-row-actions {
		display: flex;
		justify-content: flex-end;
		align-items: center;
		gap: 6px;
		flex-wrap: wrap;
	}

	.cu-row .cu-row-actions {
		position: absolute;
		inset-inline-end: 8px;
		top: 50%;
		transform: translateY(-50%);
		flex-wrap: nowrap;
		padding: 4px 4px 4px 24px;
		background: linear-gradient(90deg, transparent, var(--cu-surface) 24px);
		opacity: 0;
		transition: opacity var(--cu-motion-fast);
	}

	.cu-row-head span:nth-child(5) {
		display: none;
	}

	.cu-row:hover .cu-row-actions,
	.cu-row:focus-within .cu-row-actions,
	.cu-row-actions.cu-force-visible {
		opacity: 1;
	}

	.cu-trash-origin {
		display: block;
		color: var(--cu-muted);
		font-size: 11px;
		margin-top: 2px;
	}

	.cu-empty {
		display: grid;
		place-items: center;
		text-align: center;
		min-height: 260px;
		padding: 24px;
	}

	.cu-empty h2 {
		font-size: 16px;
		font-weight: 600;
		color: var(--cu-text);
		margin: 0 0 8px;
	}

	.cu-empty p {
		color: var(--cu-muted);
		max-width: 36ch;
		margin: 0 auto 20px;
		font-size: 13px;
	}

	.cu-empty-actions {
		display: flex;
		justify-content: center;
		gap: 8px;
	}

	.cu-drop-overlay {
		position: fixed;
		inset: 0;
		z-index: 90;
		display: grid;
		place-items: center;
		pointer-events: none;
		background: color-mix(in srgb, var(--cu-overlay) 70%, transparent);
		outline: 2px dashed var(--cu-seal-text);
		outline-offset: -12px;
	}

	.cu-drop-hint {
		display: flex;
		align-items: center;
		gap: 10px;
		padding: 14px 22px;
		background: var(--cu-surface);
		border: 1px solid var(--cu-line-strong);
		border-radius: var(--cu-radius-lg);
		box-shadow: var(--cu-shadow);
		font-size: 15px;
		font-weight: 600;
	}
</style>
