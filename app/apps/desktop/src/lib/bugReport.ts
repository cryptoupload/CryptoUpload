export const FEEDBACK_EMAIL = 'info@cryptoupload.org';
export const ISSUES_URL = 'https://github.com/cryptoupload/CryptoUpload/issues/new';

export const REPORT_TEXT_MAX = 1000;

export type ReportKind = 'bug' | 'idea';

export interface ReportFacts {
	version: string;
	os: string;
	arch: string;
	language: string;
}

export interface Report {
	kind: ReportKind;
	text: string;
	facts: ReportFacts | null;
}

const TITLE_MAX = 70;

export function reportTitle(kind: ReportKind, text: string): string {
	const first = text.trim().split('\n')[0].trim();
	const cut = first.length > TITLE_MAX ? `${first.slice(0, TITLE_MAX - 1)}…` : first;
	return `${kind === 'bug' ? 'Bug' : 'Idea'}: ${cut}`; // i18n-ignore
}

export function factsLine(f: ReportFacts): string {
	return `CryptoUpload ${f.version} · ${f.os} ${f.arch} · language ${f.language}`; // i18n-ignore
}

export function reportBody(r: Report): string {
	const text = r.text.trim().slice(0, REPORT_TEXT_MAX);
	return r.facts ? `${text}\n\n---\n${factsLine(r.facts)}` : text;
}

export function mailtoUrl(r: Report): string {
	return `mailto:${FEEDBACK_EMAIL}?subject=${encodeURIComponent(reportTitle(r.kind, r.text))}&body=${encodeURIComponent(reportBody(r))}`;
}

export function issueUrl(r: Report): string {
	const label = r.kind === 'bug' ? 'bug' : 'enhancement';
	return `${ISSUES_URL}?labels=${label}&title=${encodeURIComponent(reportTitle(r.kind, r.text))}&body=${encodeURIComponent(reportBody(r))}`;
}

export function reportText(r: Report): string {
	return `${reportTitle(r.kind, r.text)}\n\n${reportBody(r)}`;
}
