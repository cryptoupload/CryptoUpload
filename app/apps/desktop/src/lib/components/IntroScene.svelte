<script lang="ts">
	import { i18n } from '$lib/i18n/index.svelte';
	import { onMount } from 'svelte';
	import { finishIntroScene } from '$lib/state/intro.svelte';

	const TOTAL_MS = 6200;
	let timer: ReturnType<typeof setTimeout> | undefined;
	let done = false;

	function skip() {
		if (done) return;
		done = true;
		clearTimeout(timer);
		finishIntroScene();
	}

	function handleKey(e: KeyboardEvent) {
		if (e.key === 'Escape' || e.key === ' ' || e.key === 'Enter') {
			e.preventDefault();
			skip();
		}
	}

	onMount(() => {
		if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) {
			skip();
			return;
		}
		timer = setTimeout(skip, TOTAL_MS);
		return () => clearTimeout(timer);
	});
</script>

<svelte:window onkeydown={handleKey} />

<!-- svelte-ignore a11y_click_events_have_key_events -->
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="cu-intro" onclick={skip}>
	<div class="cu-intro-bar cu-intro-bar-t"></div>
	<div class="cu-intro-bar cu-intro-bar-b"></div>

	<div class="cu-intro-stage">
		<div class="cu-intro-glow"></div>
		<svg class="cu-intro-vault" viewBox="0 0 200 200" fill="none" stroke="currentColor" stroke-linecap="round">
			<circle class="cu-intro-d" style="--t: .5s" cx="100" cy="100" r="92" stroke-width="3" pathLength="1" />
			<circle class="cu-intro-d" style="--t: .8s" cx="100" cy="100" r="78" stroke-width="1.2" pathLength="1" />
			<circle class="cu-intro-d" style="--t: 1s" cx="100" cy="100" r="50" stroke-width="1.2" pathLength="1" />
			<g class="cu-intro-bolts">
				{#each [0, 45, 90, 135, 180, 225, 270, 315] as angle (angle)}
					<line
						class="cu-intro-bolt"
						transform="rotate({angle} 100 100)"
						x1="100"
						y1="11"
						x2="100"
						y2="19"
						stroke-width="5"
					/>
				{/each}
			</g>
			<circle class="cu-intro-dial" cx="100" cy="100" r="66" stroke-width="5" stroke-dasharray="3 31.56" />
			<circle class="cu-intro-d" style="--t: 1.4s" cx="100" cy="100" r="34" stroke-width="2" pathLength="1" />
			<path
				class="cu-intro-d"
				style="--t: 1.6s"
				d="M100 120V82M84 97l16-16 16 16"
				stroke-width="3.5"
				stroke-linejoin="round"
				pathLength="1"
			/>
		</svg>

		<p class="cu-intro-phrase p1">{i18n.t.intro.line1}</p>
		<p class="cu-intro-phrase p2">{i18n.t.intro.line2}</p>

		<div class="cu-intro-lockup">
			<svg class="cu-i-mark" width="56" height="56"><use href="#cu-i-mark" /></svg>
			<div class="cu-intro-word">CryptoUpload</div>
			<div class="cu-intro-tag">{i18n.t.intro.tagline}</div>
		</div>
	</div>

	<button type="button" class="cu-intro-skip" onclick={skip}>{i18n.t.intro.skip}</button>
</div>

<style>
	.cu-intro {
		position: fixed;
		inset: 0;
		z-index: 9998;
		display: grid;
		place-items: center;
		background: #02050b;
		color: #eef5ff;
		overflow: hidden;
	}

	.cu-intro-bar {
		position: absolute;
		left: 0;
		right: 0;
		height: 42%;
		background: #01030a;
		z-index: 2;
	}
	.cu-intro-bar-t {
		top: 0;
		animation: cu-intro-bar-t 1.4s cubic-bezier(0.7, 0, 0.2, 1) 0.2s both;
	}
	.cu-intro-bar-b {
		bottom: 0;
		animation: cu-intro-bar-b 1.4s cubic-bezier(0.7, 0, 0.2, 1) 0.2s both;
	}

	.cu-intro-stage {
		position: relative;
		width: min(640px, 90vw);
		display: flex;
		flex-direction: column;
		align-items: center;
	}

	.cu-intro-glow {
		position: absolute;
		left: 50%;
		top: 96px;
		width: 420px;
		height: 420px;
		margin: -210px 0 0 -210px;
		border-radius: 50%;
		background: radial-gradient(circle, rgba(61, 100, 164, 0.7), rgba(61, 100, 164, 0.2) 40%, transparent 68%);
		opacity: 0;
		transform: scale(0.3);
		animation: cu-intro-glow-up 1.4s cubic-bezier(0.2, 0.7, 0.2, 1) 3.6s forwards;
	}

	.cu-intro-vault {
		width: 200px;
		height: 200px;
		color: #cfe0f7;
		transform-origin: left center;
		animation: cu-intro-swing 0.9s cubic-bezier(0.6, 0, 0.2, 1) 3.6s forwards;
	}
	.cu-intro-d {
		stroke-dasharray: 1;
		stroke-dashoffset: 1;
		animation: cu-intro-draw 1.1s cubic-bezier(0.4, 0, 0.2, 1) var(--t, 0.5s) both;
	}
	.cu-intro-bolts {
		color: #6c93c9;
	}
	.cu-intro-bolt {
		opacity: 0;
		animation:
			cu-intro-bolt-in 0.5s ease 1.9s forwards,
			cu-intro-bolt-out 0.6s cubic-bezier(0.6, 0, 0.3, 1) 3.3s forwards;
	}
	.cu-intro-dial {
		color: #6c93c9;
		opacity: 0;
		transform-origin: 100px 100px;
		transform-box: view-box;
		animation:
			cu-intro-bolt-in 0.5s ease 1.9s forwards,
			cu-intro-spin 1.1s cubic-bezier(0.6, 0, 0.3, 1) 2.4s both;
	}

	.cu-intro-phrase {
		position: absolute;
		top: 232px;
		margin: 0;
		text-align: center;
		font: 400 26px/1.3 var(--cu-font-display);
		color: #dbe7f8;
		opacity: 0;
	}

	.cu-intro-phrase.p1 {
		animation: cu-intro-phrase 2s ease 1.1s both;
	}

	.cu-intro-phrase.p2 {
		animation: cu-intro-phrase 1s ease 3.2s both;
	}

	.cu-intro-lockup {
		position: absolute;
		top: 40px;
		text-align: center;
		opacity: 0;
		animation: cu-intro-fade-up 0.9s cubic-bezier(0.2, 0.7, 0.2, 1) 4.6s both;
	}
	.cu-i-mark {
		color: #eef5ff;
	}
	.cu-intro-word {
		font: 400 44px/1 var(--cu-font-display);
		margin-top: 12px;
	}
	.cu-intro-tag {
		font: 400 14px var(--cu-font-ui);
		color: #8fa0b8;
		margin-top: 10px;
		opacity: 0;
		animation: cu-intro-fade-up 0.8s ease 5.1s forwards;
	}

	.cu-intro-skip {
		position: absolute;
		inset-inline-end: 20px;
		bottom: 20px;
		z-index: 5;
		background: none;
		border: 1px solid rgba(214, 226, 245, 0.25);
		color: #8fa0b8;
		border-radius: var(--cu-radius-sm);
		padding: 6px 14px;
		font-size: 13px;
		cursor: pointer;
	}
	.cu-intro-skip:hover {
		color: #fff;
	}

	@keyframes cu-intro-bar-t {
		to {
			transform: translateY(-101%);
		}
	}
	@keyframes cu-intro-bar-b {
		to {
			transform: translateY(101%);
		}
	}
	@keyframes cu-intro-draw {
		from {
			stroke-dashoffset: 1;
		}
		to {
			stroke-dashoffset: 0;
		}
	}
	@keyframes cu-intro-bolt-in {
		to {
			opacity: 1;
		}
	}
	@keyframes cu-intro-bolt-out {
		to {
			transform: translateY(9px);
			opacity: 0;
		}
	}
	@keyframes cu-intro-spin {
		from {
			transform: rotate(0);
		}
		to {
			transform: rotate(120deg);
		}
	}
	@keyframes cu-intro-swing {
		0% {
			transform: none;
			opacity: 1;
		}
		70% {
			opacity: 1;
		}
		100% {
			transform: rotateY(-86deg);
			opacity: 0;
		}
	}
	@keyframes cu-intro-glow-up {
		0% {
			opacity: 0;
			transform: scale(0.3);
		}
		55% {
			opacity: 0.9;
		}
		100% {
			opacity: 0;
			transform: scale(1.6);
		}
	}
	@keyframes cu-intro-phrase {
		0% {
			opacity: 0;
			transform: translateY(8px);
		}
		20% {
			opacity: 1;
			transform: none;
		}
		78% {
			opacity: 1;
		}
		100% {
			opacity: 0;
			transform: translateY(-4px);
		}
	}
	@keyframes cu-intro-fade-up {
		from {
			opacity: 0;
			transform: translateY(10px) scale(0.98);
		}
		to {
			opacity: 1;
			transform: none;
		}
	}
</style>
