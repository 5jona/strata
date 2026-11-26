import { useToast } from '@gojona/ui';
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { StrataTransport } from '../../transport/httpWsTransport.ts';
import type {
  ContainerJobProgressPayload,
  ContainerStatusSnapshot,
  DistroEntry,
  DistroMirror,
} from '../../types/protocol.ts';
import { STRATA_METHODS } from '../../types/rpc.ts';
import type { InstallFormValues } from './InstallConfigForm.tsx';
import {
  type InstallSourceMode,
  isRawDiskImageFilename,
  parsePublicKeysInput,
  suggestAvailableSshPort,
  suggestContainerId,
  validateContainerIdInput,
  validateHttpsUrlInput,
  validateSha256Input,
  validateSshPortInput,
} from './installHelpers.ts';

/** Internal workflow hook managing catalog fetch, validation, and create/import/clone submits. */
export function useInstallWorkflow(
  transport: StrataTransport | null,
  containers: ContainerStatusSnapshot[],
  onRefresh: () => Promise<void> | void,
  runAction: <T>(
    label: string,
    action: () => Promise<T>,
    successMessage?: string,
  ) => Promise<T | null>,
) {
  const { pushToast } = useToast();
  const [sourceMode, setSourceMode] = useState<InstallSourceMode>('online');
  const [mirror, setMirror] = useState<DistroMirror>('tuna');
  const [customMirrorUrl, setCustomMirrorUrl] = useState('');
  const [catalog, setCatalog] = useState<DistroEntry[]>([]);
  const [loadingCatalog, setLoadingCatalog] = useState(false);
  const [selectedDistro, setSelectedDistro] = useState<DistroEntry | null>(null);
  const [sourceUrl, setSourceUrl] = useState('');
  const [expectedSha256, setExpectedSha256] = useState('');
  const [selectedFile, setSelectedFile] = useState<File | null>(null);
  const [uploadProgress, setUploadProgress] = useState<{ loaded: number; total: number } | null>(
    null,
  );
  const [localDoneJob, setLocalDoneJob] = useState<ContainerJobProgressPayload | null>(null);
  const [cloneSourceId, setCloneSourceId] = useState(containers[0]?.id ?? '');
  const abortRef = useRef<AbortController | null>(null);
  const containersRef = useRef(containers);
  containersRef.current = containers;

  const [form, setForm] = useState<InstallFormValues>(() => {
    const id = suggestContainerId('ubuntu-noble', containers);
    return {
      containerId: id,
      hostname: id,
      hostnameLinked: true,
      sizeGb: 16,
      enableSsh: false,
      sshPort: String(suggestAvailableSshPort(containers)),
      sshPubKeysRaw: '',
      autoStart: true,
      setDefault: containers.length === 0,
      hostBridge: true,
      noatime: true,
      l5DiagnosticMounts: false,
      syncAndroidDns: true,
      extraCharDevs: ['uinput'],
    };
  });

  useEffect(() => {
    if (containers.length > 0 && !containers.some((c) => c.id === cloneSourceId)) {
      setCloneSourceId(containers[0].id);
    }
  }, [cloneSourceId, containers]);

  const loadCatalog = useCallback(
    async (refresh = false) => {
      if (!transport) return;
      if (mirror === 'custom' && !customMirrorUrl.trim()) return;
      setLoadingCatalog(true);
      try {
        const res = await transport.invoke(STRATA_METHODS.DISTRO_CATALOG, {
          mirror,
          ...(mirror === 'custom' ? { customUrl: customMirrorUrl.trim() } : {}),
          refresh,
        });
        setCatalog(res.entries);
        setSelectedDistro((prev) => {
          const matched = prev ? res.entries.find((e) => e.id === prev.id) : undefined;
          const next = matched ?? res.entries[0] ?? null;
          if (next && !prev) {
            const suggested = suggestContainerId(
              `${next.os}-${next.release}`,
              containersRef.current,
            );
            setForm((f) => ({
              ...f,
              containerId: suggested,
              ...(f.hostnameLinked ? { hostname: suggested } : {}),
            }));
          }
          return next;
        });
      } catch (err) {
        pushToast(
          `加载发行版目录失败: ${err instanceof Error ? err.message : String(err)}`,
          'warning',
        );
      } finally {
        setLoadingCatalog(false);
      }
    },
    [customMirrorUrl, mirror, pushToast, transport],
  );

  useEffect(() => {
    if (mirror !== 'custom') void loadCatalog(false);
  }, [loadCatalog, mirror]);

  const isRawImg =
    (sourceMode === 'local' &&
      selectedFile !== null &&
      isRawDiskImageFilename(selectedFile.name)) ||
    (sourceMode === 'url' && isRawDiskImageFilename(sourceUrl));

  const idError = validateContainerIdInput(form.containerId, containers);
  const sshPortError = validateSshPortInput(form.enableSsh, form.sshPort, containers);
  const urlOrShaError = useMemo(() => {
    if (sourceMode === 'url') {
      const urlErr = validateHttpsUrlInput(sourceUrl, '镜像直链 URL');
      if (urlErr) return urlErr;
    }
    if (sourceMode === 'online' && mirror === 'custom') {
      const customErr = validateHttpsUrlInput(customMirrorUrl, '自定义镜像站 URL');
      if (customErr) return customErr;
    }
    if (sourceMode === 'url' || sourceMode === 'local') {
      return validateSha256Input(expectedSha256);
    }
    return null;
  }, [customMirrorUrl, expectedSha256, mirror, sourceMode, sourceUrl]);

  const handleSubmitOnlineOrUrl = async () => {
    if (!transport || idError || sshPortError || urlOrShaError) return;
    if (sourceMode === 'online' && !selectedDistro) return;
    await runAction('创建容器任务', async () => {
      await transport.invoke(STRATA_METHODS.CONTAINER_CREATE, {
        id: form.containerId.trim(),
        ...(sourceMode === 'online' && selectedDistro
          ? { distroId: selectedDistro.id, mirror }
          : {}),
        ...(sourceMode === 'online' && mirror === 'custom'
          ? { customUrl: customMirrorUrl.trim() }
          : {}),
        ...(sourceMode === 'url' ? { sourceUrl: sourceUrl.trim() } : {}),
        ...(expectedSha256.trim() ? { expectedSha256: expectedSha256.trim() } : {}),
        sizeMb: form.sizeGb * 1024,
        hostname: form.hostname.trim() || form.containerId.trim(),
        enableSsh: form.enableSsh,
        ...(form.enableSsh ? { sshPort: Number(form.sshPort) } : {}),
        sshPublicKeys: form.enableSsh ? parsePublicKeysInput(form.sshPubKeysRaw) : [],
        autoStart: form.autoStart,
        setDefault: form.setDefault,
        hostBridge: form.hostBridge,
        extraCharDevs: form.extraCharDevs,
        l5DiagnosticMounts: form.l5DiagnosticMounts,
        syncAndroidDns: form.syncAndroidDns,
        noatime: form.noatime,
      });
      pushToast(`已提交安装任务：${form.containerId.trim()}`, 'info');
    });
  };

  const handleSubmitClone = async () => {
    if (!transport || !cloneSourceId || idError || sshPortError) return;
    const targetId = form.containerId.trim();
    await runAction('克隆容器', async () => {
      await transport.invoke(STRATA_METHODS.CONTAINER_CLONE, {
        sourceId: cloneSourceId,
        targetId,
        hostname: form.hostname.trim() || targetId,
        ...(form.enableSsh ? { sshPort: Number(form.sshPort) } : {}),
      });
      if (form.setDefault) {
        await transport.invoke(STRATA_METHODS.CONTAINER_SET_DEFAULT, { id: targetId });
      }
      if (form.autoStart) {
        await transport.invoke(STRATA_METHODS.CONTAINER_START, { container: targetId });
      }
      const now = Date.now();
      setLocalDoneJob({
        jobId: `clone-${now}`,
        container: targetId,
        kind: 'clone',
        state: 'completed',
        stage: 'completed',
        bytesDone: 1,
        bytesTotal: 1,
        percent: 100,
        message: `已从 ${cloneSourceId} 克隆并就绪`,
        startedAtMs: now,
        updatedAtMs: now,
      });
      pushToast(`容器克隆完成：${targetId}`, 'success');
      await onRefresh();
    });
  };

  const handleUploadLocal = async () => {
    if (!transport || !selectedFile || idError || sshPortError || urlOrShaError) return;
    const ac = new AbortController();
    abortRef.current = ac;
    setLocalDoneJob(null);
    setUploadProgress({ loaded: 0, total: selectedFile.size });
    try {
      const result = await transport.uploadImage({
        containerId: form.containerId.trim(),
        file: selectedFile,
        sizeMb: form.sizeGb * 1024,
        hostname: form.hostname.trim() || form.containerId.trim(),
        enableSsh: form.enableSsh,
        ...(form.enableSsh ? { sshPort: Number(form.sshPort) } : {}),
        sshPublicKeys: form.enableSsh ? parsePublicKeysInput(form.sshPubKeysRaw) : [],
        ...(expectedSha256.trim() ? { expectedSha256: expectedSha256.trim() } : {}),
        autoStart: form.autoStart,
        setDefault: form.setDefault,
        hostBridge: form.hostBridge,
        extraCharDevs: form.extraCharDevs,
        l5DiagnosticMounts: form.l5DiagnosticMounts,
        syncAndroidDns: form.syncAndroidDns,
        noatime: form.noatime,
        signal: ac.signal,
        onProgress: (loaded, total) => setUploadProgress({ loaded, total }),
      });
      setLocalDoneJob(result);
      pushToast(`镜像导入完成：${form.containerId.trim()}`, 'success');
      setSelectedFile(null);
      await onRefresh();
    } catch (err) {
      pushToast(err instanceof Error ? err.message : String(err), 'warning');
    } finally {
      abortRef.current = null;
      setUploadProgress(null);
    }
  };

  return {
    sourceMode,
    setSourceMode,
    mirror,
    setMirror,
    customMirrorUrl,
    setCustomMirrorUrl,
    catalog,
    loadingCatalog,
    selectedDistro,
    setSelectedDistro,
    sourceUrl,
    setSourceUrl,
    expectedSha256,
    setExpectedSha256,
    selectedFile,
    setSelectedFile,
    uploadProgress,
    localDoneJob,
    setLocalDoneJob,
    cloneSourceId,
    setCloneSourceId,
    abortRef,
    form,
    setForm,
    loadCatalog,
    isRawImg,
    idError,
    sshPortError,
    urlOrShaError,
    handleSubmitOnlineOrUrl,
    handleSubmitClone,
    handleUploadLocal,
  };
}
