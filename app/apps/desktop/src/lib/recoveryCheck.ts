export function pickChallengeIndices(count: number, total: number): number[] {
	const n = Math.min(count, total);
	const seen = new Set<number>();
	const buf = new Uint32Array(1);
	while (seen.size < n) {
		crypto.getRandomValues(buf);
		seen.add(buf[0] % total);
	}
	return [...seen].sort((a, b) => a - b);
}

export function checkAnswers(words: string[], indices: number[], answers: string[]): boolean {
	if (indices.length !== answers.length) return false;
	return indices.every((idx, i) => {
		const expected = words[idx]?.trim().toLowerCase();
		const actual = answers[i]?.trim().toLowerCase();
		return expected !== undefined && expected === actual;
	});
}
