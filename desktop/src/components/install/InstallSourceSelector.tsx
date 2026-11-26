import { Badge, Button, Card, Input, SegmentedControl, StatusBanner } from '@gojona/ui';
import { Copy, FolderUp, Globe, Link2, RefreshCw, Search } from 'lucide-react';
import { useState } from 'react';
import { formatBytes } from '../../lib/format.ts';
import type { ContainerStatusSnapshot, DistroEntry, DistroMirror } from '../../types/protocol.ts';
import { LocalImportDropzone } from '../LocalImportDropzone.tsx';
import type { InstallSourceMode } from './installHelpers.ts';

export interface InstallSourceSelectorProps {
  sourceMode: InstallSourceMode;
  onSourceModeChange: (mode: InstallSourceMode) => void;
  mirror: DistroMirror;
  onMirrorChange: (mirror: DistroMirror) => void;
  customMirrorUrl: string;
  onCustomMirrorUrlChange: (url: string) => void;
  catalog: DistroEntry[];
  loadingCatalog: boolean;
  selectedDistro: DistroEntry | null;
  onSelectDistro: (entry: DistroEntry) => void;
  onRefreshCatalog: () => void;
  sourceUrl: string;
  onSourceUrlChange: (url: string) => void;
  expectedSha256: string;
  onExpectedSha256Change: (sha: string) => void;
  selectedFile: File | null;
  uploadProgress: { loaded: number; total: number } | null;
  onSelectFile: (file: File) => void;
  onCancelUpload: () => void;
  containers: ContainerStatusSnapshot[];
  cloneSourceId: string;
  onCloneSourceIdChange: (id: string) => void;
}

/** Unified 4-source container provisioning selector (Online / HTTPS URL / Local File / Clone). */
export function InstallSourceSelector({
  sourceMode,
  onSourceModeChange,
  mirror,
  onMirrorChange,
  customMirrorUrl,
  onCustomMirrorUrlChange,
  catalog,
  loadingCatalog,
  selectedDistro,
  onSelectDistro,
  onRefreshCatalog,
  sourceUrl,
  onSourceUrlChange,
  expectedSha256,
  onExpectedSha256Change,
  selectedFile,
  uploadProgress,
  onSelectFile,
  onCancelUpload,
  containers,
  cloneSourceId,
  onCloneSourceIdChange,
}: InstallSourceSelectorProps) {
  const [osFilter, setOsFilter] = useState('all');
  const [searchQuery, setSearchQuery] = useState('');

  const osFamilies = ['all', 'Ubuntu', 'Debian', 'Alpine', 'Arch'];
  const q = searchQuery.trim().toLowerCase();
  const filteredCatalog = catalog.filter((item) => {
    if (osFilter !== 'all' && !item.os.toLowerCase().includes(osFilter.toLowerCase())) return false;
    if (!q) return true;
    return (
      item.os.toLowerCase().includes(q) ||
      item.release.toLowerCase().includes(q) ||
      item.releaseTitle.toLowerCase().includes(q)
    );
  });

  return (
    <div className="flex flex-col gap-3">
      <SegmentedControl
        value={sourceMode}
        onValueChange={onSourceModeChange}
        options={[
          { value: 'online', label: '在线发行版', icon: <Globe className="size-3.5" /> },
          { value: 'url', label: '远程直链拉取', icon: <Link2 className="size-3.5" /> },
          { value: 'local', label: '本地文件导入', icon: <FolderUp className="size-3.5" /> },
          { value: 'clone', label: '从已有容器克隆', icon: <Copy className="size-3.5" /> },
        ]}
      />

      {sourceMode === 'online' && (
        <div className="flex flex-col gap-3">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <SegmentedControl
              size="sm"
              value={mirror}
              onValueChange={onMirrorChange}
              options={[
                { value: 'tuna', label: '清华 TUNA（国内推荐）' },
                { value: 'official', label: 'LinuxContainers 官方' },
                { value: 'custom', label: '自定义镜像站' },
              ]}
            />
            <Button
              size="sm"
              variant="secondary"
              disabled={loadingCatalog}
              onClick={onRefreshCatalog}
            >
              <RefreshCw className={`size-3.5 ${loadingCatalog ? 'animate-spin' : ''}`} />
              刷新目录
            </Button>
          </div>

          {mirror === 'custom' && (
            <div className="flex items-center gap-2">
              <Input
                mono
                className="flex-1"
                value={customMirrorUrl}
                placeholder="输入 Simplestreams 镜像根地址，如 https://mirrors.example.com/lxc-images"
                onChange={(e) => onCustomMirrorUrlChange(e.target.value)}
              />
              <Button
                size="sm"
                variant="primary"
                disabled={loadingCatalog}
                onClick={onRefreshCatalog}
              >
                加载目录
              </Button>
            </div>
          )}

          <div className="flex flex-wrap items-center justify-between gap-2">
            <div className="flex flex-wrap items-center gap-1">
              {osFamilies.map((family) => (
                <Button
                  key={family}
                  size="sm"
                  variant={osFilter === family ? 'pill' : 'ghost'}
                  onClick={() => setOsFilter(family)}
                >
                  {family === 'all' ? '全部发行版' : family}
                </Button>
              ))}
            </div>
            <div className="relative w-60">
              <Search className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-[var(--text-secondary)]" />
              <Input
                className="pl-8"
                value={searchQuery}
                placeholder="搜索系统名或代号…"
                onChange={(e) => setSearchQuery(e.target.value)}
              />
            </div>
          </div>

          <div className="grid grid-cols-1 gap-2.5 sm:grid-cols-2 lg:grid-cols-3">
            {filteredCatalog.map((item) => {
              const selected = selectedDistro?.id === item.id;
              return (
                <button
                  key={item.id}
                  type="button"
                  onClick={() => onSelectDistro(item)}
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
                    <div className="flex items-center gap-1">
                      {(item.id.includes('ubuntu:noble') ||
                        item.id.includes('debian:bookworm')) && (
                        <Badge variant="success">推荐</Badge>
                      )}
                      <Badge variant={selected ? 'accent' : 'neutral'}>{item.arch}</Badge>
                    </div>
                  </div>
                  <div className="text-xs text-[var(--text-secondary)]">{item.releaseTitle}</div>
                  <div className="mt-1 flex items-center justify-between font-mono text-[11px] text-[var(--text-secondary)]">
                    <span>{item.version}</span>
                    <span>{formatBytes(item.sizeBytes)}</span>
                  </div>
                </button>
              );
            })}
          </div>
        </div>
      )}

      {sourceMode === 'url' && (
        <Card className="flex flex-col gap-3 p-4">
          <div className="flex flex-col gap-1">
            <span className="text-xs font-medium text-[var(--text-secondary)]">
              ARM64 RootFS / 镜像 HTTPS 下载直链
            </span>
            <Input
              mono
              value={sourceUrl}
              placeholder="https://example.com/rootfs-arm64.tar.xz 或 .tar.gz / .tar.zst / .img.gz"
              onChange={(e) => onSourceUrlChange(e.target.value)}
            />
          </div>
          <div className="flex flex-col gap-1">
            <span className="text-xs font-medium text-[var(--text-secondary)]">
              预期 SHA256 校验值（可选）
            </span>
            <Input
              mono
              value={expectedSha256}
              placeholder="留空则跳过校验；填写 64 位 SHA256 十六进制串"
              onChange={(e) => onExpectedSha256Change(e.target.value)}
            />
          </div>
        </Card>
      )}

      {sourceMode === 'local' && (
        <div className="flex flex-col gap-2.5">
          <LocalImportDropzone
            selectedFile={selectedFile}
            uploadProgress={uploadProgress}
            onSelectFile={onSelectFile}
            onCancelUpload={onCancelUpload}
          />
          <Card className="flex flex-col gap-1 p-3.5">
            <span className="text-xs font-medium text-[var(--text-secondary)]">
              本地文件 SHA256 校验值（可选）
            </span>
            <Input
              mono
              value={expectedSha256}
              placeholder="留空跳过校验；填入 64 位 SHA256 可在流式导入完成前校验完整性"
              onChange={(e) => onExpectedSha256Change(e.target.value)}
            />
          </Card>
        </div>
      )}

      {sourceMode === 'clone' && (
        <Card className="flex flex-col gap-3 p-4">
          {containers.length === 0 ? (
            <StatusBanner
              tone="info"
              title="暂无可克隆的现有容器"
              description="请先通过在线发行版或本地导入创建至少一个容器。"
            />
          ) : (
            <>
              <span className="text-xs font-medium text-[var(--text-secondary)]">
                选择要克隆的源容器
              </span>
              <div className="flex flex-wrap gap-2">
                {containers.map((c) => (
                  <Button
                    key={c.id}
                    size="sm"
                    variant={cloneSourceId === c.id ? 'pill' : 'secondary'}
                    onClick={() => onCloneSourceIdChange(c.id)}
                  >
                    {c.id} ({c.hostname})
                  </Button>
                ))}
              </div>
              <p className="text-xs text-[var(--text-secondary)]">
                克隆将保留源容器的已安装工具链与系统文件，并生成独立的新磁盘镜像与主机名。
              </p>
            </>
          )}
        </Card>
      )}
    </div>
  );
}
