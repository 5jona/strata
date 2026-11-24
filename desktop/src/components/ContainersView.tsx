import { Button, copyTextWithFallback, EmptyState, Input, MetricTile, useToast } from '@gojona/ui';
import { Box, HardDrive, Plus, RefreshCw, Search, ServerCog, Shield } from 'lucide-react';
import { useState } from 'react';
import { formatBytes, formatSshCommand, formatUptime } from '../lib/format.ts';
import type { StrataTransport } from '../transport/httpWsTransport.ts';
import { tauriDeviceBridge } from '../transport/tauriBridge.ts';
import type { DeviceProfile } from '../types/device.ts';
import type {
  ContainerServiceRuntime,
  ContainerStatusSnapshot,
  DaemonSummarySnapshot,
} from '../types/protocol.ts';
import { STRATA_METHODS } from '../types/rpc.ts';
import { ContainerCard } from './ContainerCard.tsx';
import { type ContainerModalMode, ContainerModals } from './ContainerModals.tsx';

export interface ContainersViewProps {
  device: DeviceProfile | null;
  transport: StrataTransport | null;
  daemonStatus: DaemonSummarySnapshot | null;
  containers: ContainerStatusSnapshot[];
  servicesByContainer: Record<string, ContainerServiceRuntime[]>;
  loading: boolean;
  onRefresh: () => Promise<void>;
  onOpenTerminal: (containerId: string) => void;
  onNavigateInstall: () => void;
  runAction: <T>(label: string, fn: () => Promise<T>) => Promise<T | null>;
}

/**
 * Primary desktop overview displaying KPI tiles and progressive-disclosure container cards.
 */
export function ContainersView({
  device,
  transport,
  daemonStatus,
  containers,
  servicesByContainer,
  loading,
  onRefresh,
  onOpenTerminal,
  onNavigateInstall,
  runAction,
}: ContainersViewProps) {
  const { pushToast } = useToast();
  const [query, setQuery] = useState('');
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [modal, setModal] = useState<ContainerModalMode>(null);

  const filtered = containers.filter(
    (c) =>
      c.id.toLowerCase().includes(query.toLowerCase()) ||
      c.hostname.toLowerCase().includes(query.toLowerCase()),
  );

  const totalAllocated = containers.reduce((sum, c) => sum + c.disk.imageAllocatedBytes, 0);
  const runningCount = containers.filter((c) => c.phase === 'running').length;
  const totalRunningServices = Object.values(servicesByContainer)
    .flat()
    .filter((s) => s.status === 'running').length;

  const withBusy = async (id: string, label: string, fn: () => Promise<unknown>) => {
    setBusyId(id);
    try {
      await runAction(label, fn);
      await onRefresh();
    } finally {
      setBusyId(null);
    }
  };

  const resolveSshTarget = (c: ContainerStatusSnapshot) => ({
    user: device?.sshUser || 'root',
    host:
      c.primaryIpv4 && c.primaryIpv4 !== '0.0.0.0' ? c.primaryIpv4 : (device?.host ?? '127.0.0.1'),
    port: c.sshPort,
    remotePath: '/root',
    alias: `strata-${c.id}`,
  });

  return (
    <div className="flex h-full flex-col gap-4 overflow-y-auto p-5 strata-scroll">
      {/* Header */}
      <div className="flex flex-wrap items-center justify-between gap-3">
        <div>
          <h1 className="text-lg font-semibold text-[var(--text-primary)]">
            容器实例 · {device?.label ?? '未选择设备'}
          </h1>
          <p className="text-xs text-[var(--text-secondary)]">
            一键启停、连接终端或 VS Code Remote-SSH，展开卡片可执行空间回收、扩容与备份
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Button size="sm" variant="secondary" disabled={loading} onClick={() => void onRefresh()}>
            <RefreshCw className={`size-3.5 ${loading ? 'animate-spin' : ''}`} />
            刷新
          </Button>
          <Button size="sm" variant="primary" onClick={onNavigateInstall}>
            <Plus className="size-3.5" />
            新建 / 导入容器
          </Button>
        </div>
      </div>

      {/* KPI Strip */}
      <div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
        <MetricTile
          label="运行中容器"
          value={`${runningCount}/${containers.length}`}
          icon={<Box className="size-4 text-[var(--accent)]" />}
          statusLabel={runningCount > 0 ? '活跃' : '空闲'}
          statusVariant={runningCount > 0 ? 'success' : 'neutral'}
        />
        <MetricTile
          label="托管后台服务"
          value={totalRunningServices}
          unit="个运行中"
          icon={<ServerCog className="size-4 text-[var(--accent)]" />}
        />
        <MetricTile
          label="磁盘实际占用"
          value={formatBytes(totalAllocated)}
          icon={<HardDrive className="size-4 text-[var(--accent)]" />}
        />
        <MetricTile
          label="stratad 运行时长"
          value={daemonStatus ? formatUptime(daemonStatus.uptimeMs) : '—'}
          hint={daemonStatus ? `PID ${daemonStatus.pid}` : undefined}
          icon={<Shield className="size-4 text-[var(--accent)]" />}
        />
      </div>

      {/* Search bar */}
      <div className="relative max-w-sm">
        <Search className="pointer-events-none absolute top-1/2 left-2.5 size-3.5 -translate-y-1/2 text-[var(--text-secondary)]" />
        <Input
          className="pl-8"
          value={query}
          placeholder="搜索容器名称或主机名…"
          aria-label="搜索容器"
          onChange={(e) => setQuery(e.target.value)}
        />
      </div>

      {/* Container List */}
      {filtered.length === 0 ? (
        <EmptyState
          icon={<Box className="size-8" />}
          title={containers.length === 0 ? '尚无容器实例' : '没有匹配的容器'}
          description="前往「安装与导入」从国内镜像站拉取 ARM64 发行版，或拖拽导入本地镜像。"
          action={
            <Button size="sm" variant="primary" onClick={onNavigateInstall}>
              立即创建容器
            </Button>
          }
        />
      ) : (
        <div className="flex flex-col gap-3">
          {filtered.map((c) => {
            const svcs = servicesByContainer[c.id] ?? [];
            const runningSvcs = svcs.filter((s) => s.status === 'running').length;
            return (
              <ContainerCard
                key={c.id}
                container={c}
                serviceCount={svcs.length}
                runningServiceCount={runningSvcs}
                expanded={expandedId === c.id}
                busy={busyId === c.id}
                onToggleExpand={() => setExpandedId((cur) => (cur === c.id ? null : c.id))}
                onStart={() =>
                  transport &&
                  void withBusy(c.id, `启动 ${c.id}`, async () => {
                    await transport.invoke(STRATA_METHODS.CONTAINER_START, { container: c.id });
                    pushToast(`容器 ${c.id} 已启动`, 'success');
                  })
                }
                onStop={() =>
                  transport &&
                  void withBusy(c.id, `停止 ${c.id}`, async () => {
                    const rep = await transport.invoke(STRATA_METHODS.CONTAINER_STOP, {
                      container: c.id,
                    });
                    pushToast(
                      `容器 ${c.id} 已停止（残留挂载 ${rep.residualMounts} / 回环 ${rep.residualLoops}）`,
                      'success',
                    );
                  })
                }
                onRestart={() =>
                  transport &&
                  void withBusy(c.id, `重启 ${c.id}`, async () => {
                    await transport.invoke(STRATA_METHODS.CONTAINER_RESTART, { container: c.id });
                    pushToast(`容器 ${c.id} 已重启`, 'success');
                  })
                }
                onOpenTerminal={() => onOpenTerminal(c.id)}
                onOpenVsCode={() => {
                  const target = resolveSshTarget(c);
                  void tauriDeviceBridge
                    .openVsCodeRemote(target)
                    .then(() => pushToast(`已调用 VS Code Remote-SSH (${c.id})`, 'info'))
                    .catch((err) =>
                      pushToast(
                        `启动 VS Code 失败: ${err instanceof Error ? err.message : String(err)}`,
                        'danger',
                      ),
                    );
                }}
                onCopySsh={() => {
                  const cmd = formatSshCommand(resolveSshTarget(c));
                  void copyTextWithFallback(cmd).then((ok) =>
                    pushToast(ok ? `已复制: ${cmd}` : '复制失败', ok ? 'success' : 'warning'),
                  );
                }}
                onCompact={() =>
                  transport &&
                  void withBusy(c.id, `空间回收 ${c.id}`, async () => {
                    const res = await transport.invoke(STRATA_METHODS.CONTAINER_COMPACT, {
                      container: c.id,
                    });
                    pushToast(`空间回收完成，释放 ${formatBytes(res.trimmedBytes)}`, 'success');
                  })
                }
                onFsck={() =>
                  transport &&
                  void withBusy(c.id, `磁盘检查 ${c.id}`, async () => {
                    const res = await transport.invoke(STRATA_METHODS.CONTAINER_FSCK, {
                      container: c.id,
                    });
                    pushToast(
                      res.exitCode === 0
                        ? `fsck 通过: ${res.output}`
                        : `fsck 返回码 ${res.exitCode}`,
                      res.exitCode === 0 ? 'success' : 'warning',
                    );
                  })
                }
                onResize={() => setModal({ kind: 'resize', container: c })}
                onClone={() => setModal({ kind: 'clone', container: c })}
                onExport={(format) => {
                  if (!transport) return;
                  void runAction(`导出 ${c.id}`, async () => {
                    await transport.downloadExport(c.id, format);
                    pushToast(`已开始下载 strata-${c.id}.${format}`, 'info');
                  });
                }}
                onConfig={() => {
                  if (!transport) return;
                  void runAction('读取容器配置', async () => {
                    const res = await transport.invoke(STRATA_METHODS.CONTAINER_CONFIG_GET, {
                      container: c.id,
                    });
                    setModal({ kind: 'config', container: c, config: res.config });
                  });
                }}
                onDelete={() => setModal({ kind: 'delete', container: c })}
              />
            );
          })}
        </div>
      )}

      <ContainerModals
        modal={modal}
        busy={busyId !== null}
        onClose={() => setModal(null)}
        onResize={async (id, sizeMb) => {
          if (!transport) return;
          await withBusy(id, '扩容镜像', async () => {
            const res = await transport.invoke(STRATA_METHODS.CONTAINER_RESIZE, { id, sizeMb });
            pushToast(`镜像已扩容至 ${formatBytes(res.newBytes)}`, 'success');
          });
        }}
        onClone={async (sourceId, targetId, sshPort, hostname) => {
          if (!transport) return;
          await withBusy(sourceId, '克隆容器', async () => {
            await transport.invoke(STRATA_METHODS.CONTAINER_CLONE, {
              sourceId,
              targetId,
              sshPort,
              hostname,
            });
            pushToast(`已克隆容器 ${targetId}`, 'success');
          });
        }}
        onSaveConfig={async (patch) => {
          if (!transport) return;
          await withBusy(patch.container ?? 'main', '保存容器配置', async () => {
            await transport.invoke(STRATA_METHODS.CONTAINER_CONFIG_PATCH, patch);
            pushToast('容器配置已保存', 'success');
          });
        }}
        onDelete={async (id, deleteImage) => {
          if (!transport) return;
          await withBusy(id, '删除容器', async () => {
            await transport.invoke(STRATA_METHODS.CONTAINER_DELETE, {
              id,
              deleteImage,
              force: true,
            });
            pushToast(`容器 ${id} 已删除`, 'info');
          });
        }}
      />
    </div>
  );
}
