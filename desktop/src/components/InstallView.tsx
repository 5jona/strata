import {
  Badge,
  Button,
  Card,
  Input,
  ProgressBar,
  SegmentedControl,
  SettingRow,
  Slider,
  StatusBanner,
  Switch,
  useToast,
} from '@gojona/ui';
import { Download, FolderUp, Globe, RefreshCw, XCircle } from 'lucide-react';
import { useCallback, useEffect, useRef, useState } from 'react';
import { formatBytes } from '../lib/format.ts';
import type { StrataTransport } from '../transport/httpWsTransport.ts';
import type { ContainerJobProgressPayload, DistroEntry, DistroMirror } from '../types/protocol.ts';
import { STRATA_METHODS } from '../types/rpc.ts';
import { LocalImportDropzone } from './LocalImportDropzone.tsx';

export interface InstallViewProps {
  transport: StrataTransport | null;
  activeJobs: ContainerJobProgressPayload[];
  onRefresh: () => Promise<void>;
  runAction: <T>(label: string, fn: () => Promise<T>) => Promise<T | null>;
}

/**
 * Unified container provisioning screen: Online ARM64 Distro Gallery (TUNA / Official)
 * plus Drag-and-Drop streaming local archive/image import.
 */
export function InstallView({ transport, activeJobs, onRefresh, runAction }: InstallViewProps) {
  const { pushToast } = useToast();
  const [sourceMode, setSourceMode] = useState<'online' | 'local'>('online');
  const [mirror, setMirror] = useState<DistroMirror>('tuna');
  const [catalog, setCatalog] = useState<DistroEntry[]>([]);
  const [loadingCatalog, setLoadingCatalog] = useState(false);
  const [selectedDistro, setSelectedDistro] = useState<DistroEntry | null>(null);

  const [containerId, setContainerId] = useState('ubuntu-dev');
  const [sizeGb, setSizeGb] = useState(16);
  const [sshPort, setSshPort] = useState('2224');
  const [autoStart, setAutoStart] = useState(true);
  const [hostBridge, setHostBridge] = useState(true);
  const [injectKey, setInjectKey] = useState(false);
  const [sshPubKey, setSshPubKey] = useState('');

  const [selectedFile, setSelectedFile] = useState<File | null>(null);
  const [uploadProgress, setUploadProgress] = useState<{ loaded: number; total: number } | null>(
    null,
  );
  const abortRef = useRef<AbortController | null>(null);

  const loadCatalog = useCallback(
    async (refresh = false) => {
      if (!transport) return;
      setLoadingCatalog(true);
      try {
        const res = await transport.invoke(STRATA_METHODS.DISTRO_CATALOG, { mirror, refresh });
        setCatalog(res.entries);
        if (!selectedDistro && res.entries[0]) setSelectedDistro(res.entries[0]);
      } catch (err) {
        pushToast(
          `加载发行版目录失败: ${err instanceof Error ? err.message : String(err)}`,
          'warning',
        );
      } finally {
        setLoadingCatalog(false);
      }
    },
    [mirror, pushToast, selectedDistro, transport],
  );

  useEffect(() => {
    void loadCatalog(false);
  }, [loadCatalog]);

  const handleInstallOnline = async () => {
    if (!transport || !selectedDistro || !containerId.trim()) return;
    await runAction('创建容器任务', async () => {
      await transport.invoke(STRATA_METHODS.CONTAINER_CREATE, {
        id: containerId.trim(),
        distroId: selectedDistro.id,
        mirror,
        sizeMb: sizeGb * 1024,
        hostname: containerId.trim(),
        sshPort: Number(sshPort) || 2224,
        sshPublicKeys: injectKey && sshPubKey.trim() ? [sshPubKey.trim()] : [],
        autoStart,
        hostBridge,
        noatime: true,
      });
      pushToast(`已提交安装任务：${containerId.trim()}`, 'info');
    });
  };

  const handleUploadLocal = async () => {
    if (!transport || !selectedFile || !containerId.trim()) return;
    const ac = new AbortController();
    abortRef.current = ac;
    setUploadProgress({ loaded: 0, total: selectedFile.size });
    try {
      await transport.uploadImage({
        containerId: containerId.trim(),
        file: selectedFile,
        sizeMb: sizeGb * 1024,
        hostname: containerId.trim(),
        sshPort: Number(sshPort) || 2224,
        autoStart,
        hostBridge,
        noatime: true,
        signal: ac.signal,
        onProgress: (loaded, total) => setUploadProgress({ loaded, total }),
      });
      pushToast(`镜像导入完成：${containerId.trim()}`, 'success');
      setSelectedFile(null);
      await onRefresh();
    } catch (err) {
      pushToast(err instanceof Error ? err.message : String(err), 'warning');
    } finally {
      abortRef.current = null;
      setUploadProgress(null);
    }
  };

  return (
    <div className="flex h-full flex-col gap-4 overflow-y-auto p-5 strata-scroll">
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold text-[var(--text-primary)]">安装与导入容器</h1>
          <p className="text-xs text-[var(--text-secondary)]">
            从清华 TUNA / LinuxContainers 镜像站一键拉取 ARM64 发行版，或拖拽流式上传本地包
          </p>
        </div>
        <SegmentedControl
          value={sourceMode}
          onValueChange={setSourceMode}
          options={[
            { value: 'online', label: '在线发行版商店', icon: <Globe className="size-3.5" /> },
            { value: 'local', label: '本地镜像导入', icon: <FolderUp className="size-3.5" /> },
          ]}
        />
      </div>

      {activeJobs.length > 0 && (
        <div className="flex flex-col gap-2">
          {activeJobs.map((job) => (
            <Card key={job.jobId} className="flex flex-col gap-2 p-3.5">
              <div className="flex items-center justify-between gap-2">
                <div className="flex items-center gap-2">
                  <span className="text-sm font-medium text-[var(--text-primary)]">
                    {job.container}
                  </span>
                  <Badge
                    variant={
                      job.state === 'completed'
                        ? 'success'
                        : job.state === 'failed'
                          ? 'danger'
                          : 'accent'
                    }
                  >
                    {job.stage}
                  </Badge>
                </div>
                {job.state === 'running' && transport && (
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() =>
                      void transport.invoke(STRATA_METHODS.CONTAINER_JOB_CANCEL, {
                        jobId: job.jobId,
                      })
                    }
                  >
                    <XCircle className="size-3.5" />
                    取消
                  </Button>
                )}
              </div>
              <ProgressBar
                value={job.percent ?? 0}
                showValue
                label={job.message ?? '处理中…'}
                tone={job.state === 'failed' ? 'danger' : 'accent'}
              />
            </Card>
          ))}
        </div>
      )}

      {sourceMode === 'online' ? (
        <div className="flex flex-col gap-3">
          <div className="flex items-center justify-between gap-2">
            <SegmentedControl
              size="sm"
              value={mirror}
              onValueChange={setMirror}
              options={[
                { value: 'tuna', label: '清华 TUNA（国内推荐）' },
                { value: 'official', label: 'LinuxContainers 官方' },
              ]}
            />
            <Button
              size="sm"
              variant="secondary"
              disabled={loadingCatalog}
              onClick={() => void loadCatalog(true)}
            >
              <RefreshCw className={`size-3.5 ${loadingCatalog ? 'animate-spin' : ''}`} />
              刷新目录
            </Button>
          </div>
          <div className="grid grid-cols-1 gap-2.5 sm:grid-cols-2 lg:grid-cols-3">
            {catalog.map((item) => {
              const selected = selectedDistro?.id === item.id;
              return (
                <button
                  key={item.id}
                  type="button"
                  onClick={() => {
                    setSelectedDistro(item);
                    setContainerId(`${item.os.toLowerCase().replace(/\s+/g, '-')}-${item.release}`);
                  }}
                  className={`flex flex-col gap-1.5 rounded-xl border p-3.5 text-left transition-colors ${
                    selected
                      ? 'border-[var(--accent)] bg-[var(--accent)]/12'
                      : 'border-[var(--border-hairline)] bg-[var(--surface-card)] hover:border-[var(--accent)]/40'
                  }`}
                >
                  <div className="flex items-center justify-between gap-2">
                    <span className="text-sm font-semibold text-[var(--text-primary)]">
                      {item.os}
                    </span>
                    <Badge variant={selected ? 'accent' : 'neutral'}>{item.arch}</Badge>
                  </div>
                  <div className="text-xs text-[var(--text-secondary)]">{item.releaseTitle}</div>
                  <div className="mt-1 flex items-center justify-between font-mono text-[11px] text-[var(--text-secondary)]">
                    <span>v{item.version}</span>
                    <span>{formatBytes(item.sizeBytes)}</span>
                  </div>
                </button>
              );
            })}
          </div>
        </div>
      ) : (
        <LocalImportDropzone
          selectedFile={selectedFile}
          uploadProgress={uploadProgress}
          onSelectFile={(file) => {
            setSelectedFile(file);
            setContainerId(file.name.replace(/\.(tar\.(gz|xz|zst)|img(\.gz)?)$/i, '').slice(0, 24));
          }}
          onCancelUpload={() => abortRef.current?.abort()}
        />
      )}

      <Card className="flex flex-col gap-3 p-4">
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
          <div className="flex flex-col gap-1">
            <span className="text-xs text-[var(--text-secondary)]">容器 ID / 主机名</span>
            <Input mono value={containerId} onChange={(e) => setContainerId(e.target.value)} />
          </div>
          <div className="flex flex-col gap-1">
            <span className="text-xs text-[var(--text-secondary)]">Dropbear SSH 端口</span>
            <Input mono value={sshPort} onChange={(e) => setSshPort(e.target.value)} />
          </div>
          <div className="flex items-end">
            <Slider
              className="w-full"
              label="稀疏镜像上限"
              min={8}
              max={128}
              step={4}
              unit=" GiB"
              value={sizeGb}
              onValueChange={setSizeGb}
            />
          </div>
        </div>

        <div className="grid grid-cols-1 gap-2 sm:grid-cols-3">
          <SettingRow
            variant="plain"
            title="开机自启"
            control={<Switch checked={autoStart} onCheckedChange={setAutoStart} />}
          />
          <SettingRow
            variant="plain"
            title="宿主机桥接"
            control={<Switch checked={hostBridge} onCheckedChange={setHostBridge} />}
          />
          <SettingRow
            variant="plain"
            title="预注入 SSH 公钥"
            control={<Switch checked={injectKey} onCheckedChange={setInjectKey} />}
          />
        </div>

        {injectKey && (
          <Input
            mono
            value={sshPubKey}
            placeholder="ssh-ed25519 AAAA... user@pc"
            onChange={(e) => setSshPubKey(e.target.value)}
          />
        )}

        <div className="flex items-center justify-between gap-2 pt-1">
          <StatusBanner
            className="flex-1"
            tone="info"
            title="自动完成 ColorOS 兼容配置（Dropbear 公钥鉴权 + APT::Sandbox::User root）"
          />
          {sourceMode === 'online' ? (
            <Button
              variant="primary"
              disabled={!transport || !selectedDistro || !containerId.trim()}
              onClick={() => void handleInstallOnline()}
            >
              <Download className="size-4" />
              开始安装容器
            </Button>
          ) : (
            <Button
              variant="primary"
              disabled={
                !transport || !selectedFile || !containerId.trim() || uploadProgress !== null
              }
              onClick={() => void handleUploadLocal()}
            >
              <FolderUp className="size-4" />
              流式上传并创建
            </Button>
          )}
        </div>
      </Card>
    </div>
  );
}
