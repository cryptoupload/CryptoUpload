let enabled = true;

export function initNotifications(initialEnabled: boolean): void {
	enabled = initialEnabled;
}

export function setNotificationsEnabled(value: boolean): void {
	enabled = value;
}

export function notificationsEnabled(): boolean {
	return enabled;
}
