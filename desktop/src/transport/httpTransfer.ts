import type { ContainerJobProgressPayload } from '../types/protocol.ts';

export interface ImportUploadOptions {
  containerId: string;
  file: File;
  sizeMb: number;
  hostname?: string;
  sshPort?: number;
  autoStart: boolean;
  hostBridge: boolean;
  noatime: boolean;
  onProgress?: (loaded: number, total: number) => void;
  signal?: AbortSignal;
}

/** Streams a local `.tar.*` or `.img` file to `stratad` `/api/images/import` with XHR progress. */
export function uploadContainerImage(
  baseUrl: string,
  token: string,
  options: ImportUploadOptions,
): Promise<ContainerJobProgressPayload> {
  return new Promise((resolve, reject) => {
    const qs = new URLSearchParams({
      id: options.containerId,
      filename: options.file.name,
      sizeMb: String(options.sizeMb),
      autoStart: String(options.autoStart),
      hostBridge: String(options.hostBridge),
      noatime: String(options.noatime),
    });
    if (options.hostname) qs.set('hostname', options.hostname);
    if (options.sshPort) qs.set('sshPort', String(options.sshPort));

    const xhr = new XMLHttpRequest();
    xhr.open('POST', `${baseUrl}/api/images/import?${qs.toString()}`);
    if (token) {
      xhr.setRequestHeader('Authorization', `Bearer ${token}`);
    }
    xhr.setRequestHeader('Content-Type', 'application/octet-stream');

    xhr.upload.onprogress = (ev) => {
      if (ev.lengthComputable) {
        options.onProgress?.(ev.loaded, ev.total);
      }
    };

    const onAbort = () => xhr.abort();
    options.signal?.addEventListener('abort', onAbort, { once: true });

    xhr.onload = () => {
      options.signal?.removeEventListener('abort', onAbort);
      if (xhr.status >= 200 && xhr.status < 300) {
        try {
          resolve(JSON.parse(xhr.responseText) as ContainerJobProgressPayload);
        } catch {
          reject(new Error('导入响应 JSON 解析失败'));
        }
      } else {
        reject(new Error(`导入失败 (HTTP ${xhr.status}): ${xhr.responseText}`));
      }
    };
    xhr.onerror = () => {
      options.signal?.removeEventListener('abort', onAbort);
      reject(new Error('镜像上传网络中断'));
    };
    xhr.onabort = () => {
      reject(new Error('用户取消上传'));
    };
    xhr.send(options.file);
  });
}

/** Downloads a `.tar.gz` or `.img.gz` export stream from `stratad` to the local browser. */
export async function downloadContainerExport(
  baseUrl: string,
  headers: Record<string, string>,
  fetchImpl: typeof fetch,
  containerId: string,
  format: 'tar.gz' | 'img.gz',
): Promise<void> {
  const url = `${baseUrl}/api/containers/${encodeURIComponent(containerId)}/export?format=${format}`;
  const resp = await fetchImpl(url, { headers });
  if (!resp.ok) {
    throw new Error(`导出失败 (HTTP ${resp.status})`);
  }
  const blob = await resp.blob();
  const objectUrl = URL.createObjectURL(blob);
  const link = document.createElement('a');
  link.href = objectUrl;
  link.download = `strata-${containerId}.${format}`;
  document.body.appendChild(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(objectUrl), 10_000);
}
