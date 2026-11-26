import { copyTextWithFallback, useToast } from '@gojona/ui';
import { useState } from 'react';
import { formatBytes, formatSshCommand } from '../lib/format.ts';
import { buildPreviewSshTarget, resolveLaunchSshTarget } from '../lib/sshTarget.ts';
import type { StrataTransport } from '../transport/httpWsTransport.ts';
import { tauriDeviceBridge } from '../transport/tauriBridge.ts';
import type { DeviceProfile } from '../types/device.ts';
import type {
  ContainerConfig,
  ContainerServiceRuntime,
  ContainerStatusSnapshot,
  DaemonSummarySnapshot,
} from '../types/protocol.ts';
import { STRATA_METHODS } from '../types/rpc.ts';
import { ContainerCard } from './ContainerCard.tsx';
import { ContainerConfigModal } from './ContainerConfigModal.tsx';
import { type ContainerModalMode, ContainerModals } from './ContainerModals.tsx';
import { ContainersEmptyState, ContainersHeader } from './ContainersEmptyState.tsx';
import { ContainersKpiStrip } from './ContainersKpiStrip.tsx';

export interface ContainersViewProps {
  device: DeviceProfile | null;
  transport: StrataTransport | null;
  daemonStatus: DaemonSummarySnapshot | null;
  containers: ContainerStatusSnapshot[];
  selectedContainerId?: string;
  onSelectContainer: (id: string | undefined) => void;
  servicesByContainer: Record<string, ContainerServiceRuntime[]>;
  configsByContainer: Record<string, ContainerConfig>;
  loading: boolean;
  onRefresh: () => Promise<void>;
  onOpenTerminal: (containerId: string) => void;
  onNavigateInstall: () => void;
  runAction: <T>(label: string, fn: () => Promise<T>) => Promise<T | null>;
}

/**
 * Primary desktop overview displaying KPI tiles, zero-container onboarding, and container cards.
 */
export function ContainersView({
  device,
  transport,
  daemonStatus,
  containers,
  onSelectContainer,
  servicesByContainer,
  configsByContainer,
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
  const [configTarget, setConfigTarget] = useState<{
    container: ContainerStatusSnapshot;
    config: ContainerConfig;
  } | null>(null);

  const filtered = containers.filter(
    (c) =>
      c.id.toLowerCase().includes(query.toLowerCase()) ||
      c.hostname.toLowerCase().includes(query.toLowerCase()),
  );
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

  const handleOpenVsCode = (c: ContainerStatusSnapshot) => {
    void runAction('打开 VS Code 远程开发', async () => {
      const { target, forwardedLocalPort } = await resolveLaunchSshTarget(device, c);
      await tauriDeviceBridge.openVsCodeRemote(target);
      pushToast(
        forwardedLocalPort
          ? `已建立 ADB SSH 转发 (127.0.0.1:${forwardedLocalPort}) 并拉起 VS Code`
          : `已调用 VS Code Remote-SSH (${c.id})`,
        'info',
      );
    });
  };

  return (
    <div className="flex h-full flex-col gap-4 overflow-y-auto p-5 strata-scroll">
      <ContainersHeader
        deviceLabel={device?.label ?? '未选择设备'}
        loading={loading}
        showSearch={containers.length > 0}
        query={query}
        onQueryChange={setQuery}
        onRefresh={() => void onRefresh()}
        onNavigateInstall={onNavigateInstall}
      />

      <ContainersKpiStrip
        containers={containers}
        runningServiceCount={totalRunningServices}
        daemonStatus={daemonStatus}
      />

      {filtered.length === 0 && !loading ? (
        <ContainersEmptyState
          hasAnyContainers={containers.length > 0}
          onNavigateInstall={onNavigateInstall}
          onClearQuery={() => setQuery('')}
        />
      ) : (
        <div className="flex flex-col gap-3">
          {filtered.map((c) => {
            const svcs = servicesByContainer[c.id] ?? [];
            const runningSvcs = svcs.filter((s) => s.status === 'running').length;
            const cfg = configsByContainer[c.id];
            const isDefault = daemonStatus?.defaultContainer
              ? daemonStatus.defaultContainer === c.id
              : c.isDefault;
            return (
              <ContainerCard
                key={c.id}
                container={c}
                isDefault={isDefault}
                hasUinput={cfg ? cfg.extraCharDevs.includes('uinput') : undefined}
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
                    const clean = rep.residualMounts === 0 && rep.residualLoops === 0;
                    pushToast(
                      clean
                        ? `容器 ${c.id} 已安全停止`
                        : `容器 ${c.id} 已停止（残留挂载 ${rep.residualMounts} / 回环 ${rep.residualLoops}）`,
                      clean ? 'success' : 'warning',
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
                onOpenVsCode={() => handleOpenVsCode(c)}
                onCopySsh={() => {
                  const cmd = formatSshCommand(buildPreviewSshTarget(device, c));
                  void copyTextWithFallback(cmd).then((ok) =>
                    pushToast(ok ? `已复制：${cmd}` : '复制失败', ok ? 'success' : 'warning'),
                  );
                }}
                onSetDefault={() =>
                  transport &&
                  void withBusy(c.id, `设为默认调试容器 ${c.id}`, async () => {
                    await transport.invoke(STRATA_METHODS.CONTAINER_SET_DEFAULT, { id: c.id });
                    onSelectContainer(c.id);
                    pushToast(`已将 ${c.id} 设为默认调试容器`, 'success');
                  })
                }
                onCompact={() =>
                  transport &&
                  void withBusy(c.id, `回收磁盘空间 ${c.id}`, async () => {
                    const res = await transport.invoke(STRATA_METHODS.CONTAINER_COMPACT, {
                      container: c.id,
                    });
                    pushToast(`磁盘空间回收完成，释放 ${formatBytes(res.trimmedBytes)}`, 'success');
                  })
                }
                onFsck={() =>
                  transport &&
                  void withBusy(c.id, `磁盘修复检查 ${c.id}`, async () => {
                    const res = await transport.invoke(STRATA_METHODS.CONTAINER_FSCK, {
                      container: c.id,
                    });
                    pushToast(
                      res.exitCode <= 1
                        ? '磁盘修复检查完成，文件系统正常'
                        : `磁盘修复检查发现异常（退出码 ${res.exitCode}）`,
                      res.exitCode <= 1 ? 'success' : 'warning',
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
                onSyncDns={() =>
                  transport &&
                  void withBusy(c.id, `同步 DNS ${c.id}`, async () => {
                    await transport.invoke(STRATA_METHODS.CONTAINER_SYNC_DNS, { container: c.id });
                    pushToast(`已将 Android DNS 同步至 ${c.id}`, 'success');
                  })
                }
                onConfig={() => {
                  if (!transport) return;
                  void runAction('读取容器配置', async () => {
                    const res = await transport.invoke(STRATA_METHODS.CONTAINER_CONFIG_GET, {
                      container: c.id,
                    });
                    setConfigTarget({ container: c, config: res.config });
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

      <ContainerConfigModal
        open={configTarget !== null}
        container={configTarget?.container ?? null}
        config={configTarget?.config ?? null}
        busy={busyId !== null}
        onClose={() => setConfigTarget(null)}
        onSave={async (patch) => {
          if (!transport) return;
          await withBusy(patch.container ?? 'main', '保存容器配置', async () => {
            await transport.invoke(STRATA_METHODS.CONTAINER_CONFIG_PATCH, patch);
            pushToast('容器配置已保存', 'success');
          });
        }}
      />
    </div>
  );
}
