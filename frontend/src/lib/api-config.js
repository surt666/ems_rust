export const apiConfig = {
	baseUrl: import.meta.env.PUBLIC_API_BASE_URL || 'https://vp9p5wrn6f.execute-api.eu-central-1.amazonaws.com',
};

export function buildApiUrl(path) {
	const cleanPath = path.startsWith('/') ? path.slice(1) : path;
	return `${apiConfig.baseUrl}/${cleanPath}`;
}

export async function apiCommand(body) {
	const res = await fetch(`${apiConfig.baseUrl}/command`, {
		method: 'POST',
		headers: { 'Content-Type': 'application/json' },
		body: JSON.stringify(body),
	});
	const data = await res.json();
	if (!res.ok) throw new Error(data?.error?.message || res.statusText);
	return data;
}

export async function apiQuery(action, params = {}) {
	const qs = new URLSearchParams(params).toString();
	const url = `${apiConfig.baseUrl}/query/${action}${qs ? '?' + qs : ''}`;
	const res = await fetch(url);
	const data = await res.json();
	if (!res.ok) throw new Error(data?.error?.message || res.statusText);
	return data;
}
