export interface PluralForms {
	zero?: string;
	one?: string;
	two?: string;
	few?: string;
	many?: string;
	other: string;
}

export type Localized<T> = T extends { other: string }
	? PluralForms
	: T extends string
		? string
		: { [K in keyof T]: Localized<T[K]> };

export type Partial2<T> = T extends PluralForms
	? PluralForms
	: T extends string
		? string
		: { [K in keyof T]?: Partial2<T[K]> };
