import { invoke } from '@tauri-apps/api/core';
import { describeMessage } from '$lib/errors';

export type Remote =
	| {
			provider: 's3';
			name: string;
			endpoint: string;
			bucket: string;
			prefix: string;
			allowInsecureHttp: boolean;
	  }
	| {
			provider: 'sftp';
			name: string;
			host: string;
			port: number;
			username: string;
			remoteRoot: string;
	  }
	| { provider: 'gdrive'; name: string; folderId: string }
	| { provider: 'dropbox'; name: string; clientId: string }
	| {
			provider: 'onedrive';
			name: string;
			clientId: string;
			tenant: string;
			hasClientSecret: boolean;
			rootKind: OneDriveRootKind;
	  }
	| {
			provider: 'webdav';
			name: string;
			baseUrl: string;
			username: string;
			rootPath: string;
			allowInsecureHttp: boolean;
	  };

export type OneDriveRootKind = 'app_folder' | 'drive_root';

export interface HostKeyInfo {
	keyType: string;
	fingerprint: string;
}

export type RemotesStatus = 'missing' | 'legacy' | 'current';

export interface S3PresetInfo {
	id: string;
	label: string;
	hasEndpoint: boolean;
	regions: string[];
	pathStyle: boolean;
	note: string | null;
}

export async function s3Presets(): Promise<S3PresetInfo[]> {
	const presets = await invoke<S3PresetInfo[]>('s3_presets');
	return presets.map((p) => ({ ...p, label: describeMessage(p.label) }));
}

export function s3PresetEndpoint(presetId: string, region: string): Promise<string | null> {
	return invoke('s3_preset_endpoint', { presetId, region });
}

export function remotesStatus(): Promise<RemotesStatus> {
	return invoke('remotes_status');
}

export function remotesMigrateLegacy(oldPassword: string): Promise<string[]> {
	return invoke('remotes_migrate_legacy', { oldPassword });
}

export function remotesFileDelete(): Promise<void> {
	return invoke('remotes_file_delete');
}

export function remoteListNames(): Promise<string[]> {
	return invoke('remote_list_names');
}

export function remoteUnlock(name: string, password: string): Promise<Remote> {
	return invoke('remote_unlock', { name, password });
}

export function remoteLock(name: string): Promise<void> {
	return invoke('remote_lock', { name });
}

export type S3ObjectLockMode = 'governance' | 'compliance';

export function remoteAddS3(
	name: string,
	endpoint: string,
	region: string,
	bucket: string,
	prefix: string,
	pathStyle: boolean,
	accessKey: string,
	secretKey: string,
	password: string,
	objectLockMode: S3ObjectLockMode | null = null,
	objectLockDays: number | null = null,
	allowInsecureHttp = false,
): Promise<void> {
	return invoke('remote_add_s3', {
		name,
		endpoint,
		region,
		bucket,
		prefix,
		pathStyle,
		accessKey,
		secretKey,
		password,
		objectLockMode,
		objectLockDays,
		allowInsecureHttp,
	});
}

export function isPlainHttp(url: string): boolean {
	return url.trim().toLowerCase().startsWith('http://');
}

export function remoteAllowInsecureHttp(name: string): Promise<Remote> {
	return invoke('remote_allow_insecure_http', { name });
}

export function remoteProbeSftpHostKey(host: string, port: number): Promise<HostKeyInfo> {
	return invoke('remote_probe_sftp_host_key', { host, port });
}

export function remoteAddSftp(
	name: string,
	host: string,
	port: number,
	username: string,
	authMethod: 'password' | 'key',
	authPassword: string | null,
	authKeyPath: string | null,
	authKeyPassphrase: string | null,
	remoteRoot: string,
	hostKeyType: string,
	hostKeyFingerprint: string,
	password: string,
): Promise<void> {
	return invoke('remote_add_sftp', {
		name,
		host,
		port,
		username,
		authMethod,
		authPassword,
		authKeyPath,
		authKeyPassphrase,
		remoteRoot,
		hostKeyType,
		hostKeyFingerprint,
		password,
	});
}

export function remoteAddGdrive(
	name: string,
	clientId: string,
	clientSecret: string,
	folderName: string,
	password: string,
): Promise<void> {
	return invoke('remote_add_gdrive', { name, clientId, clientSecret, folderName, password });
}

export function remoteAddDropbox(
	name: string,
	clientId: string,
	password: string,
): Promise<void> {
	return invoke('remote_add_dropbox', { name, clientId, password });
}

export function remoteAddOneDrive(
	name: string,
	clientId: string,
	clientSecret: string | null,
	tenant: string,
	rootKind: OneDriveRootKind,
	password: string,
): Promise<void> {
	return invoke('remote_add_onedrive', {
		name,
		clientId,
		clientSecret,
		tenant,
		rootKind,
		password,
	});
}

export function remoteAddWebDav(
	name: string,
	baseUrl: string,
	username: string,
	webdavPassword: string,
	rootPath: string,
	allowInsecureHttp: boolean,
	password: string,
): Promise<void> {
	return invoke('remote_add_webdav', {
		name,
		baseUrl,
		username,
		webdavPassword,
		rootPath,
		allowInsecureHttp,
		password,
	});
}

export function remoteRemove(name: string, password: string): Promise<void> {
	return invoke('remote_remove', { name, password });
}

export function remoteSweep(name: string, olderThanDays: number): Promise<string[]> {
	return invoke('remote_sweep', { name, olderThanDays });
}
