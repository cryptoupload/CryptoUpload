<script lang="ts">
	import { i18n } from '$lib/i18n/index.svelte';
	import { onMount } from 'svelte';
	import { sceneMotionOff } from '$lib/state/theme.svelte';

	let { ondone }: { ondone: () => void } = $props();

	const skip = sceneMotionOff();

	onMount(() => {
		const t = setTimeout(ondone, skip ? 0 : 1400);
		return () => clearTimeout(t);
	});
</script>

{#if !skip}
	<div class="cu-opening" aria-hidden="true">
		<div class="cu-opening-streak"></div>
		<div class="cu-opening-flood"></div>
		<div class="cu-opening-lb top"></div>
		<div class="cu-opening-lb bottom"></div>
		<div class="cu-opening-text">{i18n.t.opening.vaultOpen}</div>
	</div>
{/if}

<style>
	.cu-opening {
		position: fixed;
		inset: 0;
		z-index: 50;
		pointer-events: none;
		overflow: hidden;
	}

	.cu-opening-streak {
		position: absolute;
		left: 0;
		right: 0;
		top: calc(50% - 1px);
		height: 3px;
		background: linear-gradient(90deg, transparent, #eef5ff 50%, transparent);
		box-shadow: 0 0 28px 6px rgb(141 180 232 / 0.6);
		transform: scaleX(0);
		opacity: 0;
		animation: cu-opening-streak 0.6s ease-out 0.05s both;
	}

	.cu-opening-flood {
		position: absolute;
		inset: 0;
		background: radial-gradient(70% 60% at 50% 50%, rgb(235 244 255 / 0.95), rgb(141 180 232 / 0.5) 45%, transparent 75%);
		opacity: 0;
		animation: cu-opening-flood 0.6s ease-out 0.2s both;
	}

	.cu-opening-lb {
		position: absolute;
		left: 0;
		right: 0;
		height: 52%;
		background: #02050b;
	}

	.cu-opening-lb.top {
		top: 0;
		animation: cu-opening-bar-t 0.7s cubic-bezier(0.7, 0, 0.2, 1) 0.6s both;
	}

	.cu-opening-lb.bottom {
		bottom: 0;
		animation: cu-opening-bar-b 0.7s cubic-bezier(0.7, 0, 0.2, 1) 0.6s both;
	}

	.cu-opening-text {
		position: absolute;
		left: 0;
		right: 0;
		top: calc(50% - 20px);
		text-align: center;
		font: italic 400 32px/1 var(--cu-font-display);
		color: #eef5ff;
		opacity: 0;
		animation: cu-opening-text 0.6s ease 0.05s both;
	}

	@keyframes cu-opening-streak {
		0% {
			transform: scaleX(0);
			opacity: 0;
		}
		40% {
			transform: scaleX(1);
			opacity: 1;
		}
		100% {
			transform: scaleX(1);
			opacity: 0;
		}
	}

	@keyframes cu-opening-flood {
		0% {
			opacity: 0;
		}
		35% {
			opacity: 0.85;
		}
		100% {
			opacity: 0;
		}
	}

	@keyframes cu-opening-bar-t {
		from {
			transform: translateY(0);
		}
		to {
			transform: translateY(-101%);
		}
	}

	@keyframes cu-opening-bar-b {
		from {
			transform: translateY(0);
		}
		to {
			transform: translateY(101%);
		}
	}

	@keyframes cu-opening-text {
		0% {
			opacity: 0;
			transform: translateY(6px);
		}
		25% {
			opacity: 1;
			transform: none;
		}
		70% {
			opacity: 1;
		}
		100% {
			opacity: 0;
			transform: translateY(-4px);
		}
	}
</style>
